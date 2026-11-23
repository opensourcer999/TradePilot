//! Translating the news into the language of the screens, on demand: only the headlines on the screen and the text of the item
//! that is opened. The requests go to a free service one after the other with a gap between them (it answers "too many
//! requests" otherwise), the answers are kept in `news_translations.json` so the same news is not translated again, and a failure
//! never stops anything: the original text is shown until a translation is there. After a failure the worker asks the service
//! nothing for a while; the jobs of that time come back as skipped and are asked for again later.

use super::{App, Instant};
use crate::news::{self, Region};
use crate::storage::{data_file, read_json_file, write_json_file};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

/// The file in the data folder that keeps the translations.
const CACHE_FILE: &str = "news_translations.json";

/// The least time between two requests to the service.
const MIN_GAP: Duration = Duration::from_millis(350);

/// After a failure (the daily limit is used up, no network) no request is made for this long.
const PAUSE_AFTER_FAILURE: Duration = Duration::from_secs(300);

/// What became of one job.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// The translated text.
    Done(String),
    /// The service failed; the reason in words.
    Failed(String),
    /// Not asked, because the worker pauses after a failure; the text is asked for again later.
    Skipped,
}

type Answer = (String, Outcome);

struct Job {
    key: String,
    text: String,
    pair: &'static str,
    contact: String,
}

/// The translations that are there, those that are asked for, and the worker that asks.
#[derive(Default)]
pub(super) struct TranslationState {
    /// By language pair and original text; read from the file at the first use.
    cache: Option<HashMap<String, String>>,
    /// The keys that have been handed to the worker and not answered yet.
    queued: HashSet<String>,
    jobs: Option<Sender<Job>>,
    results: Option<Receiver<Answer>>,
    /// No request is made before this time.
    paused_until: Option<Instant>,
    /// Why the last request failed; cleared by the next success.
    pub(super) error: Option<String>,
}

/// What a translation is kept under: the language pair and the original text.
fn key_of(pair: &str, text: &str) -> String {
    format!("{pair}|{text}")
}

/// Works off the jobs one at a time: at least `MIN_GAP` between two requests, and after a failure no request for
/// `PAUSE_AFTER_FAILURE` (the jobs of that time are answered as skipped). Ends when the app takes no more answers or sends no
/// more jobs. `now` is the clock, `woken` tells the screen that an answer is there.
fn work_off(
    jobs: &Receiver<Job>,
    answers: &Sender<Answer>,
    translate: impl Fn(&Job) -> Result<String, String>,
    now: impl Fn() -> std::time::Instant,
    woken: impl Fn(),
) {
    let mut last: Option<std::time::Instant> = None;
    let mut paused_until: Option<std::time::Instant> = None;
    for job in jobs {
        let outcome = if paused_until.is_some_and(|until| now() < until) {
            Outcome::Skipped
        } else {
            if let Some(at) = last {
                std::thread::sleep(MIN_GAP.saturating_sub(now().saturating_duration_since(at)));
            }
            let result = translate(&job);
            last = Some(now());
            match result {
                Ok(text) => Outcome::Done(text),
                Err(reason) => {
                    paused_until = now().checked_add(PAUSE_AFTER_FAILURE);
                    Outcome::Failed(reason)
                }
            }
        };
        if answers.send((job.key, outcome)).is_err() {
            break;
        }
        woken();
    }
}

/// The thread that makes the requests (see `work_off`).
fn spawn_worker(ctx: &egui::Context) -> (Sender<Job>, Receiver<Answer>) {
    let (job_sender, job_receiver) = mpsc::channel::<Job>();
    let (answer_sender, answer_receiver) = mpsc::channel::<Answer>();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        work_off(
            &job_receiver,
            &answer_sender,
            |job| news::translate_text(&job.text, job.pair, &job.contact),
            std::time::Instant::now,
            || {
                ctx.request_repaint();
            },
        );
    });
    (job_sender, answer_receiver)
}

impl App {
    /// The translation of `text` of an item from `region` into the language of the screens; `None` when none is needed or it is not there yet.
    pub(super) fn translated(&self, region: Region, text: &str) -> Option<String> {
        let pair = news::language_pair(region, self.german())?;
        self.news.translation.cache.as_ref()?.get(&key_of(pair, text)).cloned()
    }

    /// Reads the cache from its file at the first use; a file that cannot be read is logged and the translations start empty.
    fn ensure_translation_cache(&mut self) {
        if self.news.translation.cache.is_some() {
            return;
        }
        let cache = match data_file(CACHE_FILE).map_or_else(|| Ok(HashMap::new()), |path| read_json_file(&path)) {
            Ok(cache) => cache,
            Err(reason) => {
                self.log(&format!("WARNING: News: {reason} - the translations start empty"));
                HashMap::new()
            }
        };
        self.news.translation.cache = Some(cache);
    }

    /// Asks for the translation of `text` unless it is there, asked for already, or the service is paused after a failure.
    pub(super) fn want_translation(&mut self, ctx: &egui::Context, region: Region, text: &str) {
        let Some(pair) = news::language_pair(region, self.german()) else { return };
        if text.trim().is_empty() {
            return;
        }
        let key = key_of(pair, text);
        let now = self.clock.instant();
        self.ensure_translation_cache();
        let contact = self.settings.translation_contact_email.clone();
        let state = &mut self.news.translation;
        let known = state.cache.as_ref().is_some_and(|cache| cache.contains_key(&key));
        if known || state.queued.contains(&key) || state.paused_until.is_some_and(|until| now < until) {
            return;
        }
        state.queued.insert(key.clone());
        if state.jobs.is_none() {
            // Tests have no network: they hand in a channel of their own or none.
            if cfg!(test) {
                return;
            }
            let (jobs, results) = spawn_worker(ctx);
            state.jobs = Some(jobs);
            state.results = Some(results);
        }
        let sent = state.jobs.as_ref().is_some_and(|jobs| jobs.send(Job { key, text: text.to_string(), pair, contact }).is_ok());
        if !sent {
            self.translation_worker_stopped();
        }
    }

    /// The worker is gone (its thread ended): what it had is forgotten, so it is asked for again, by a new worker that starts with
    /// the next request.
    fn translation_worker_stopped(&mut self) {
        let state = &mut self.news.translation;
        let lost = state.queued.len();
        state.queued.clear();
        state.jobs = None;
        state.results = None;
        self.log(&format!("WARNING: News: the translation worker stopped - {lost} request(s) are asked for again"));
    }

    /// Takes the answers that have come in: a translation goes into the cache, a failure is logged and pauses the requests for a
    /// while, a skipped job is asked for again later.
    pub(super) fn poll_translations(&mut self) {
        let Some(receiver) = &self.news.translation.results else { return };
        let mut answers: Vec<Answer> = Vec::new();
        let stopped = loop {
            match receiver.try_recv() {
                Ok(answer) => answers.push(answer),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if !answers.is_empty() {
            self.ensure_translation_cache();
        }
        let now = self.clock.instant();
        let mut failures: Vec<String> = Vec::new();
        let mut added = false;
        let state = &mut self.news.translation;
        for (key, outcome) in answers {
            state.queued.remove(&key);
            match outcome {
                Outcome::Done(text) => {
                    if let Some(cache) = state.cache.as_mut() {
                        cache.insert(key, text);
                    }
                    state.error = None;
                    added = true;
                }
                Outcome::Failed(reason) => {
                    state.error = Some(reason.clone());
                    state.paused_until = now.checked_add(PAUSE_AFTER_FAILURE);
                    failures.push(reason);
                }
                Outcome::Skipped => {}
            }
        }
        for reason in failures {
            self.log(&format!("News: translation failed: {reason}"));
        }
        if stopped {
            self.translation_worker_stopped();
        }
        if added {
            self.save_translations();
        }
    }

    /// Writes the cache to its file; of the language on the screen only what is in the news now is kept, so the file does not grow.
    fn save_translations(&mut self) {
        let to_german = self.german();
        let keep: HashSet<String> = self
            .news
            .items
            .iter()
            .filter_map(|item| news::language_pair(item.region, to_german).map(|pair| (pair, item)))
            .flat_map(|(pair, item)| [key_of(pair, &item.title), key_of(pair, &item.text)])
            .collect();
        let own_pairs = format!("{}|", news::pair_for_screens(to_german));
        let Some(cache) = self.news.translation.cache.as_mut() else { return };
        cache.retain(|key, _| !key.starts_with(&own_pairs) || keep.contains(key));
        let Some(path) = data_file(CACHE_FILE) else { return };
        if let Err(reason) = write_json_file(&path, cache) {
            self.log(&format!("WARNING: News: {reason}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use std::cell::Cell;

    fn german_app() -> App {
        let mut app = App::new(Some("test".to_string()), None);
        app.settings.lang = Lang::De;
        app
    }

    /// A news item with the headline `title`.
    fn titled(title: &str, region: Region) -> news::NewsItem {
        news::NewsItem { title: title.into(), link: title.into(), ..news::test_item(region, 0) }
    }

    /// A channel for answers, handed to the app as if a worker were running.
    fn answers_to(app: &mut App) -> Sender<Answer> {
        let (sender, receiver) = mpsc::channel::<Answer>();
        app.news.translation.results = Some(receiver);
        sender
    }

    #[test]
    fn a_translation_is_asked_for_once_and_only_for_news_not_in_the_language_of_the_screens() {
        let mut app = german_app();
        let ctx = egui::Context::default();
        app.want_translation(&ctx, Region::Us, "Fed holds rates");
        app.want_translation(&ctx, Region::Us, "Fed holds rates");
        app.want_translation(&ctx, Region::De, "Fed halt die Zinsen");
        app.want_translation(&ctx, Region::Uk, "  ");
        assert_eq!(app.news.translation.queued.len(), 1, "once, and neither a German item nor an empty text");
        assert_eq!(app.translated(Region::Us, "Fed holds rates"), None, "not there yet");
    }

    #[test]
    fn the_news_line_of_the_dashboard_asks_for_the_translation_of_its_headline() {
        let mut app = german_app();
        app.news.items = vec![titled("Stocks soar", Region::Us)];
        app.news.seen = Some(crate::app::news::SeenNews { up_to: Some(0) });
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.news_bulletin(ui));
        });
        assert!(app.news.translation.queued.contains(&key_of("en|de", "Stocks soar")), "{:?}", app.news.translation.queued);
    }
    #[test]
    fn an_answer_goes_into_the_cache_and_a_failure_pauses_the_requests() {
        let mut app = german_app();
        let ctx = egui::Context::default();
        app.want_translation(&ctx, Region::Us, "Jobs grew");
        app.want_translation(&ctx, Region::Uk, "Prices fell");
        app.news.items = vec![titled("Jobs grew", Region::Us), titled("Prices fell", Region::Uk)];
        let sender = answers_to(&mut app);
        sender.send((key_of("en|de", "Jobs grew"), Outcome::Done("Die Besch\u{e4}ftigung stieg".to_string()))).unwrap();
        app.poll_translations();
        assert_eq!(app.translated(Region::Us, "Jobs grew").as_deref(), Some("Die Besch\u{e4}ftigung stieg"));
        assert_eq!(app.translated(Region::Uk, "Jobs grew").as_deref(), Some("Die Besch\u{e4}ftigung stieg"), "a UK item is in English as well");
        assert_eq!(app.news.translation.error, None);

        sender.send((key_of("en|de", "Prices fell"), Outcome::Failed("the daily limit of the free translation service is used up".to_string()))).unwrap();
        app.poll_translations();
        assert!(app.news.translation.error.as_deref().is_some_and(|e| e.contains("daily limit")));
        app.want_translation(&ctx, Region::Us, "Something new");
        assert!(!app.news.translation.queued.contains(&key_of("en|de", "Something new")), "paused after the failure");
    }

    #[test]
    fn every_failure_is_logged_even_when_a_success_follows_and_a_skipped_job_is_asked_again_later() {
        let mut app = german_app();
        let ctx = egui::Context::default();
        app.want_translation(&ctx, Region::Us, "One");
        app.want_translation(&ctx, Region::Us, "Two");
        app.want_translation(&ctx, Region::Us, "Three");
        app.news.items = ["One", "Two", "Three"].into_iter().map(|title| titled(title, Region::Us)).collect();
        let sender = answers_to(&mut app);
        sender.send((key_of("en|de", "One"), Outcome::Failed("no network".to_string()))).unwrap();
        sender.send((key_of("en|de", "Two"), Outcome::Done("Zwei".to_string()))).unwrap();
        sender.send((key_of("en|de", "Three"), Outcome::Skipped)).unwrap();
        app.poll_translations();
        assert!(app.log_lines.iter().any(|l| l.contains("translation failed: no network")), "{:?}", app.log_lines);
        assert_eq!(app.translated(Region::Us, "Two").as_deref(), Some("Zwei"));
        assert_eq!(app.news.translation.error, None, "cleared by the success after it");
        assert!(app.news.translation.queued.is_empty(), "the skipped job is no longer waiting, so it can be asked for again");
        assert_eq!(app.translated(Region::Us, "Three"), None);
    }

    #[test]
    fn a_worker_that_stopped_is_logged_and_what_it_had_is_asked_for_again() {
        // The worker's thread ended: its answers channel is closed.
        let mut app = german_app();
        let ctx = egui::Context::default();
        app.want_translation(&ctx, Region::Us, "Lost");
        drop(answers_to(&mut app));
        app.poll_translations();
        assert!(app.log_lines.iter().any(|l| l.contains("WARNING: News: the translation worker stopped - 1 request(s)")), "{:?}", app.log_lines);
        assert!(app.news.translation.queued.is_empty() && app.news.translation.results.is_none());
        // A job cannot be handed over: the same.
        let mut app = german_app();
        let (jobs, gone) = mpsc::channel::<Job>();
        drop(gone);
        app.news.translation.jobs = Some(jobs);
        app.want_translation(&ctx, Region::Us, "Not sent");
        assert!(app.log_lines.iter().any(|l| l.contains("the translation worker stopped")), "{:?}", app.log_lines);
        assert!(app.news.translation.queued.is_empty() && app.news.translation.jobs.is_none());
    }

    #[test]
    fn the_worker_asks_nothing_for_a_while_after_a_failure_and_answers_the_waiting_jobs_as_skipped() {
        let (jobs, job_receiver) = mpsc::channel::<Job>();
        let (answer_sender, answers) = mpsc::channel::<Answer>();
        for (n, text) in ["a", "b", "c"].into_iter().enumerate() {
            jobs.send(Job { key: format!("k{n}"), text: text.to_string(), pair: "en|de", contact: String::new() }).unwrap();
        }
        drop(jobs);
        // A clock that moves on a second at every look, so the worker never has to wait for the gap.
        let clock = Cell::new(std::time::Instant::now());
        let now = || {
            let at = clock.get();
            clock.set(at + Duration::from_secs(1));
            at
        };
        let asked = Cell::new(0);
        work_off(
            &job_receiver,
            &answer_sender,
            |job| {
                asked.set(asked.get() + 1);
                if job.text == "a" {
                    Err("limit".to_string())
                } else {
                    Ok(job.text.to_uppercase())
                }
            },
            now,
            || {},
        );
        let outcomes: Vec<Answer> = answers.try_iter().collect();
        assert_eq!(
            outcomes,
            [("k0".to_string(), Outcome::Failed("limit".to_string())), ("k1".to_string(), Outcome::Skipped), ("k2".to_string(), Outcome::Skipped)]
        );
        assert_eq!(asked.get(), 1, "the service is asked only once");
    }

    #[test]
    fn the_cache_of_the_language_on_the_screen_keeps_only_what_is_in_the_news() {
        let mut app = german_app();
        app.news.items = vec![titled("Kept", Region::Us)];
        let mut cache = HashMap::new();
        cache.insert(key_of("en|de", "Kept"), "Behalten".to_string());
        cache.insert(key_of("en|de", "Gone"), "Weg".to_string());
        cache.insert(key_of("de|en", "Alt"), "Old".to_string());
        app.news.translation.cache = Some(cache);
        app.save_translations();
        let cache = app.news.translation.cache.as_ref().unwrap();
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key(&key_of("en|de", "Kept")) && cache.contains_key(&key_of("de|en", "Alt")));
    }
}
