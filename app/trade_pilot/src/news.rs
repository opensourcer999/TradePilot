//! Economic news from public RSS feeds, US news leading, German and British ones mixed in. The feeds are fetched at once on
//! background threads (`load`), read with a small parser that knows only what an RSS 2.0 feed needs (`parse_feed`), and the newest items
//! are kept (`latest`). Translating them into the language of the screens is done piece by piece on demand (`translate_text`).

use crate::http_error::error_text;
use chrono::{DateTime, Utc};
use std::io::Read;
use std::time::Duration;

/// How many news items are kept; the screen shows as many of them as fit (see `app::news`).
pub const SHOWN: usize = 30;

/// The most a feed may be (bytes): a larger answer is cut off.
const MAX_FEED_BYTES: u64 = 4 * 1024 * 1024;

/// Where a news item comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// United States (English).
    Us,
    /// Germany (German).
    De,
    /// United Kingdom (English).
    Uk,
}

impl Region {
    /// The short mark shown in the table.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Region::Us => "US",
            Region::De => "DE",
            Region::Uk => "UK",
        }
    }

    /// Whether the feeds of this region are written in German.
    #[must_use]
    pub fn is_german(self) -> bool {
        self == Region::De
    }
}

/// How a source delivers its news.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// An RSS 2.0 feed.
    Rss,
    /// The search of Yahoo Finance (JSON), which lists the news about a symbol.
    YahooSearch,
}

/// The sources: (name, address, region, format). Yahoo Finance is asked for the news of the S&P 500.
const FEEDS: [(&str, &str, Region, Format); 4] = [
    ("CNBC", "https://search.cnbc.com/rs/search/combinedcms/view.xml?partnerId=wrss01&id=20910258", Region::Us, Format::Rss),
    ("Yahoo Finance", "https://query1.finance.yahoo.com/v1/finance/search?q=%5EGSPC&newsCount=20&quotesCount=0", Region::Us, Format::YahooSearch),
    ("tagesschau.de", "https://www.tagesschau.de/wirtschaft/index~rss2.xml", Region::De, Format::Rss),
    ("BBC", "https://feeds.bbci.co.uk/news/business/rss.xml", Region::Uk, Format::Rss),
];

/// The names of the sources, for the line under the title.
#[must_use]
pub fn source_names() -> String {
    FEEDS.iter().map(|(name, ..)| *name).collect::<Vec<_>>().join(", ")
}

/// One news item.
#[derive(Clone, Debug, PartialEq)]
pub struct NewsItem {
    /// The headline as plain text.
    pub title: String,
    /// The text of the item as plain text; paragraphs are separated by a line break. A feed carries a summary, not the whole article.
    pub text: String,
    /// The address of the whole article.
    pub link: String,
    /// When the source published the item (UTC).
    pub published: DateTime<Utc>,
    /// The country of the source, which also gives the language of the item.
    pub region: Region,
}

/// An item for tests: from `region`, published `minute` minutes after a fixed moment, with headline and address `<region><minute>`
/// (as `US3`) and no text.
#[cfg(test)]
pub fn test_item(region: Region, minute: i64) -> NewsItem {
    NewsItem {
        title: format!("{}{minute}", region.label()),
        text: String::new(),
        link: format!("{}{minute}", region.label()),
        published: DateTime::from_timestamp(1_700_000_000 + minute * 60, 0).unwrap_or_default(),
        region,
    }
}

impl NewsItem {
    /// What identifies the item: its address, or its headline when it has none.
    #[must_use]
    pub fn key(&self) -> &str {
        if self.link.is_empty() {
            &self.title
        } else {
            &self.link
        }
    }
}

/// What a load brings: the items to show and the sources that could not be read.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The items to show, in the order of the screen.
    pub items: Vec<NewsItem>,
    /// One line per source that could not be read: its name and the reason.
    pub failed_sources: Vec<String>,
}

/// Loads every feed and keeps the newest items (US first).
/// The error is the reason in words, when no feed could be read at all.
pub fn load() -> Result<Loaded, String> {
    // All sources at once: a slow one costs its own timeout, not the sum of all of them.
    let answers: Vec<(&str, Result<Vec<NewsItem>, String>)> = std::thread::scope(|scope| {
        let running: Vec<_> = FEEDS.iter().map(|&(name, url, region, format)| (name, scope.spawn(move || fetch(url, region, format)))).collect();
        running.into_iter().map(|(name, thread)| (name, thread.join().unwrap_or_else(|_| Err("the loading thread stopped".to_string())))).collect()
    });
    let mut all = Vec::new();
    let mut failed = Vec::new();
    for (name, answer) in answers {
        match answer {
            Ok(items) => all.extend(items),
            Err(reason) => failed.push(format!("{name}: {reason}")),
        }
    }
    if all.is_empty() {
        return Err(failed.join("; "));
    }
    let items = latest(all, SHOWN);
    Ok(Loaded { items, failed_sources: failed })
}

/// Loads a source and reads its items. The error is the reason in words.
fn fetch(url: &str, region: Region, format: Format) -> Result<Vec<NewsItem>, String> {
    let response = ureq::get(url).timeout(Duration::from_secs(10)).set("User-Agent", "TradePilot").call().map_err(|e| error_text(&e))?;
    let mut body = String::new();
    response.into_reader().take(MAX_FEED_BYTES).read_to_string(&mut body).map_err(|e| e.to_string())?;
    let items = match format {
        Format::Rss => parse_feed(&body, region),
        Format::YahooSearch => parse_yahoo(&body, region),
    };
    if items.is_empty() {
        return Err("the feed has no news items".to_string());
    }
    Ok(items)
}

/// How the screen mixes the sources: this many US items, then one German or British one, and so on.
const US_PER_FOREIGN: usize = 2;

/// The newest `count` items. US news lead, with two US items to every German or British one (each group newest first), so the
/// news of the other countries reach the screen too; when one group runs out, the other fills the rest. An item that appears
/// twice (same address) counts once.
#[must_use]
pub fn latest(mut items: Vec<NewsItem>, count: usize) -> Vec<NewsItem> {
    items.sort_by_key(|item| std::cmp::Reverse(item.published));
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.key().to_string()));
    let (us, foreign): (Vec<NewsItem>, Vec<NewsItem>) = items.into_iter().partition(|item| item.region == Region::Us);
    let (mut us, mut foreign) = (us.into_iter(), foreign.into_iter());
    let mut mixed = Vec::with_capacity(count);
    while mixed.len() < count {
        let before = mixed.len();
        mixed.extend(us.by_ref().take(US_PER_FOREIGN));
        mixed.extend(foreign.next());
        if mixed.len() == before {
            break;
        }
    }
    mixed.truncate(count);
    mixed
}

/// The items of an RSS 2.0 feed. An item without a title or without a readable date is left out.
#[must_use]
pub fn parse_feed(xml: &str, region: Region) -> Vec<NewsItem> {
    blocks(xml, "item")
        .into_iter()
        .filter_map(|item| {
            let title = plain_text(&inner(item, "title")?);
            let published = DateTime::parse_from_rfc2822(inner(item, "pubDate")?.trim()).ok()?.with_timezone(&Utc);
            if title.is_empty() {
                return None;
            }
            let summary = inner(item, "description").map(|d| plain_text(&d)).unwrap_or_default();
            let content = inner(item, "content:encoded").map(|c| plain_text(&c)).unwrap_or_default();
            // The longer of the two is the fuller text; the "more" marker of a feed that links to the article is no text.
            let text = if content.chars().count() > summary.chars().count() { content } else { summary };
            let text = text.replace("[mehr]", "").replace("[more]", "").trim().to_string();
            let link = inner(item, "link").map(|l| plain_text(&l)).unwrap_or_default();
            Some(NewsItem { title, text, link, published, region })
        })
        .collect()
}

/// The news in an answer of the Yahoo Finance search. An item carries a headline, the publisher and the time, but no text; the publisher
/// stands in its place. An item without a headline or a time is left out.
#[must_use]
pub fn parse_yahoo(json: &str, region: Region) -> Vec<NewsItem> {
    let Ok(answer) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let Some(list) = answer.get("news").and_then(serde_json::Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|entry| {
            let title = entry.get("title").and_then(serde_json::Value::as_str)?.trim().to_string();
            let published = DateTime::from_timestamp(entry.get("providerPublishTime").and_then(serde_json::Value::as_i64)?, 0)?;
            if title.is_empty() {
                return None;
            }
            let link = entry.get("link").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
            let text = entry.get("publisher").and_then(serde_json::Value::as_str).filter(|p| !p.is_empty()).map(|p| format!("Source: {p}")).unwrap_or_default();
            Some(NewsItem { title, text, link, published, region })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Translation
// ---------------------------------------------------------------------------------------------------------------------

/// The free translation service `MyMemory`: no key, a small daily limit for an anonymous caller (a contact address raises it).
const TRANSLATE_URL: &str = "https://api.mymemory.translated.net/get";

/// The longest piece the service takes in one request (characters); it refuses more than 500.
const MAX_PIECE: usize = 450;

/// The language pair that turns news of other countries into the language of the screens: `en|de` for German screens, `de|en` for English.
#[must_use]
pub fn pair_for_screens(to_german: bool) -> &'static str {
    if to_german {
        "en|de"
    } else {
        "de|en"
    }
}

/// The language pair of a translation into the language of the screens; `None` when the item is in that language already.
#[must_use]
pub fn language_pair(region: Region, to_german: bool) -> Option<&'static str> {
    (region.is_german() != to_german).then(|| pair_for_screens(to_german))
}

/// `text` cut into pieces of at most `max` characters at sentence ends (a longer sentence at spaces), so each piece fits one request.
#[must_use]
pub fn pieces(text: &str, max: usize) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|next| next.is_whitespace()) {
            sentences.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current);
    }
    let mut out: Vec<String> = Vec::new();
    let mut piece = String::new();
    for part in sentences.iter().flat_map(|sentence| cut_at_spaces(sentence, max)) {
        if !piece.is_empty() && piece.chars().count() + part.chars().count() > max {
            out.push(std::mem::take(&mut piece).trim().to_string());
        }
        piece.push_str(&part);
    }
    if !piece.trim().is_empty() {
        out.push(piece.trim().to_string());
    }
    out
}

/// One sentence in parts of at most `max` characters, cut at spaces (a word longer than `max` stays whole).
fn cut_at_spaces(sentence: &str, max: usize) -> Vec<String> {
    if sentence.chars().count() <= max {
        return vec![sentence.to_string()];
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    for word in sentence.split_inclusive(char::is_whitespace) {
        if !part.is_empty() && part.chars().count() + word.chars().count() > max {
            parts.push(std::mem::take(&mut part));
        }
        part.push_str(word);
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

/// `text` translated with the language pair `pair` (`en|de`): line by line, a long line in pieces. Never more than one request at a time
/// is the caller's business (see the queue in `app::news`). The error is the reason in words.
pub fn translate_text(text: &str, pair: &str, contact: &str) -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut done: Vec<String> = Vec::new();
        for piece in pieces(line, MAX_PIECE) {
            done.push(translate_piece(&piece, pair, contact)?);
        }
        lines.push(done.join(" "));
    }
    Ok(lines.join("\n"))
}

fn translate_piece(piece: &str, pair: &str, contact: &str) -> Result<String, String> {
    let mut request = ureq::get(TRANSLATE_URL).timeout(Duration::from_secs(10)).set("User-Agent", "TradePilot").query("q", piece).query("langpair", pair);
    if !contact.trim().is_empty() {
        request = request.query("de", contact.trim());
    }
    let body = request.call().map_err(|e| error_text(&e))?.into_string().map_err(|e| e.to_string())?;
    let answer: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("the answer could not be read: {e}"))?;
    parse_translation(&answer)
}

/// The translated text in an answer of the service. The service reports an exhausted limit inside a normal answer, as a text.
fn parse_translation(answer: &serde_json::Value) -> Result<String, String> {
    let status = answer.get("responseStatus").and_then(|s| s.as_i64().or_else(|| s.as_str().and_then(|text| text.parse().ok()))).unwrap_or(200);
    let text = answer.get("responseData").and_then(|data| data.get("translatedText")).and_then(serde_json::Value::as_str).unwrap_or_default();
    if status != 200 {
        let details = answer.get("responseDetails").and_then(serde_json::Value::as_str).unwrap_or_default();
        return Err(format!("the service answered {status} {details}").trim().to_string());
    }
    if text.starts_with("MYMEMORY WARNING") {
        return Err("the daily limit of the free translation service is used up".to_string());
    }
    if text.trim().is_empty() {
        return Err("the service returned no text".to_string());
    }
    Ok(decode_entities(text).trim().to_string())
}

/// The text between the opening and the closing tag of the first element `tag` in `block`, a CDATA section unwrapped.
fn inner(block: &str, tag: &str) -> Option<String> {
    blocks(block, tag).into_iter().next().map(|text| {
        let text = text.trim();
        text.strip_prefix("<![CDATA[").and_then(|t| t.strip_suffix("]]>")).unwrap_or(text).to_string()
    })
}

/// The inner parts of every element `tag` in `xml`, in order. An element is found by its exact name; one inside another of the same name is not looked for.
fn blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let Some(after) = rest.get(at + open.len()..) else { break };
        // `<item` must not be the start of `<items`: the name ends with `>`, a space or `/`.
        let named = after.chars().next().is_some_and(|c| c == '>' || c == '/' || c.is_whitespace());
        let Some(tag_end) = after.find('>').filter(|_| named) else {
            rest = after;
            continue;
        };
        let (Some(head), Some(content)) = (after.get(..tag_end), after.get(tag_end + 1..)) else { break };
        if head.ends_with('/') {
            rest = content;
            continue;
        }
        let Some(end) = content.find(&close) else { break };
        let (Some(inner), Some(next)) = (content.get(..end), content.get(end + close.len()..)) else { break };
        found.push(inner);
        rest = next;
    }
    found
}

/// HTML as plain text: tags removed (a paragraph or line break becomes a line break), the usual character references decoded,
/// white space reduced to single spaces, empty lines dropped.
#[must_use]
pub fn plain_text(html: &str) -> String {
    let unwrapped = html.trim();
    let unwrapped = unwrapped.strip_prefix("<![CDATA[").and_then(|t| t.strip_suffix("]]>")).unwrap_or(unwrapped);
    let mut text = String::with_capacity(unwrapped.len());
    let mut chars = unwrapped.char_indices();
    while let Some((at, c)) = chars.next() {
        if c != '<' {
            text.push(c);
            continue;
        }
        let tail = unwrapped.get(at..).unwrap_or_default();
        // A tag starts with a letter, `/` or `!`; a lone "<" (as in "5 < 6") is text.
        let is_tag = tail.get(1..).and_then(|t| t.chars().next()).is_some_and(|next| next.is_ascii_alphabetic() || next == '/' || next == '!');
        let Some(end) = tail.find('>').filter(|_| is_tag) else {
            text.push(c);
            continue;
        };
        let tag = tail.get(1..end).unwrap_or_default().trim_start_matches('/');
        let name: String = tag.chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        if matches!(name.as_str(), "p" | "br" | "div" | "li" | "h1" | "h2" | "h3" | "h4") {
            text.push('\n');
        }
        // Skip the rest of the tag: what follows the "<", up to and including the ">".
        for _ in 0..tail.get(1..=end).map_or(0, |rest| rest.chars().count()) {
            chars.next();
        }
    }
    decode_entities(&text)
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The character references `&amp; &lt; &gt; &quot; &apos; &nbsp;`, `&#39;` and `&#x27;` as the characters they stand for.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or_default());
        let tail = rest.get(at..).unwrap_or_default();
        let reference = tail.find(';').filter(|end| *end <= 10).and_then(|end| tail.get(1..end));
        let decoded = reference.and_then(|name| match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|number| number.parse().ok()))
                .and_then(char::from_u32),
        });
        // Past the reference: "&", its name and ";".
        if let (Some(c), Some(next)) = (decoded, reference.and_then(|name| tail.get(name.len() + 2..))) {
            out.push(c);
            rest = next;
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::test_item as item;
    use super::*;

    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Feed</title>
      <item>
        <title>Fed holds rates &amp; signals patience</title>
        <link>https://example.com/a</link>
        <description><![CDATA[The central bank left <b>rates</b> unchanged.]]></description>
        <pubDate>Wed, 18 Nov 2026 14:09:26 GMT</pubDate>
      </item>
      <item>
        <title>Inflation cools</title>
        <link>https://example.com/b</link>
        <description>Short</description>
        <content:encoded><![CDATA[<p>Prices rose less than expected.<br/>Core inflation eased.</p>[<a href="x">mehr</a>]]]></content:encoded>
        <pubDate>Thu, 19 Nov 2026 17:54:27 +0100</pubDate>
      </item>
      <item><title>No date</title><link>https://example.com/c</link></item>
    </channel></rss>"#;

    #[test]
    fn a_feed_is_read_into_items_with_decoded_titles_plain_texts_and_utc_times() {
        let items = parse_feed(FEED, Region::Us);
        assert_eq!(items.len(), 2, "the item without a date is left out");
        assert_eq!(items[0].title, "Fed holds rates & signals patience");
        assert_eq!(items[0].text, "The central bank left rates unchanged.");
        assert_eq!(items[0].link, "https://example.com/a");
        assert_eq!(items[0].published.to_rfc3339(), "2026-11-18T14:09:26+00:00");
        assert_eq!(items[1].published.to_rfc3339(), "2026-11-19T16:54:27+00:00", "+0100 is converted");
    }

    #[test]
    fn the_yahoo_search_is_read_into_items_with_the_publisher_as_text() {
        let json = r#"{"news":[
            {"title":"Dow drops 800 points","publisher":"Yahoo Finance","link":"https://finance.yahoo.com/a","providerPublishTime":1795111725},
            {"title":"No time","publisher":"X","link":"https://finance.yahoo.com/b"},
            {"title":"","publisher":"X","link":"https://finance.yahoo.com/c","providerPublishTime":1795111000},
            {"title":"No publisher","link":"https://finance.yahoo.com/d","providerPublishTime":1795110000}]}"#;
        let items = parse_yahoo(json, Region::Us);
        assert_eq!(items.len(), 2, "an item without a time or a headline is left out");
        assert_eq!(
            (items[0].title.as_str(), items[0].text.as_str(), items[0].link.as_str()),
            ("Dow drops 800 points", "Source: Yahoo Finance", "https://finance.yahoo.com/a")
        );
        assert_eq!(items[0].published.timestamp(), 1_795_111_725);
        assert_eq!(items[1].text, "");
        assert!(parse_yahoo("not json", Region::Us).is_empty() && parse_yahoo("{}", Region::Us).is_empty());
    }

    #[test]
    fn the_fuller_text_wins_and_keeps_its_paragraphs_without_the_more_marker() {
        let items = parse_feed(FEED, Region::Us);
        assert_eq!(items[1].text, "Prices rose less than expected.\nCore inflation eased.");
    }

    #[test]
    fn the_newest_ten_come_first_and_the_same_article_counts_once() {
        let mut items = parse_feed(FEED, Region::Us);
        items.push(items[0].clone());
        let newest = latest(items, 10);
        assert_eq!(newest.iter().map(|i| i.link.as_str()).collect::<Vec<_>>(), ["https://example.com/b", "https://example.com/a"]);
        let many: Vec<NewsItem> = (0..15).map(|n| item(Region::Us, n)).collect();
        let top = latest(many, 10);
        assert_eq!((top.len(), top[0].title.as_str(), top[9].title.as_str()), (10, "US14", "US5"));
    }

    #[test]
    fn character_references_and_tags_are_turned_into_plain_text() {
        assert_eq!(plain_text("Tom &amp; Jerry&#39;s &#x41; &lt;b&gt; &unknown; &"), "Tom & Jerry's A <b> &unknown; &");
        assert_eq!(plain_text("<p>One</p><p>Two<br>Three</p>"), "One\nTwo\nThree");
        assert_eq!(plain_text("a   b\n\n  c"), "a b\nc");
        assert_eq!(plain_text("5 < 6 and 7 > 3"), "5 < 6 and 7 > 3");
    }

    #[test]
    fn a_name_that_only_starts_like_a_tag_is_not_that_tag() {
        assert_eq!(blocks("<items>x</items><item>y</item>", "item"), ["y"]);
    }

    #[test]
    fn us_news_lead_with_two_to_every_german_or_british_one_and_the_rest_fills_up() {
        let items = vec![
            item(Region::De, 50),
            item(Region::Us, 1),
            item(Region::Uk, 40),
            item(Region::Us, 2),
            item(Region::De, 45),
            item(Region::Us, 3),
            item(Region::Us, 4),
        ];
        let order: Vec<String> = latest(items.clone(), 10).into_iter().map(|i| i.title).collect();
        assert_eq!(order, ["US4", "US3", "DE50", "US2", "US1", "DE45", "UK40"], "an old US item still leads a newer German one");
        // Many US news do not push the others off: a short list still has one of three from abroad.
        let many_us: Vec<NewsItem> = (10..40).map(|minute| item(Region::Us, minute)).chain(items).collect();
        let top: Vec<String> = latest(many_us, 6).into_iter().map(|i| i.title).collect();
        assert_eq!(top, ["US39", "US38", "DE50", "US37", "US36", "DE45"]);
    }

    #[test]
    fn an_item_is_identified_by_its_address_or_else_its_headline() {
        let with_link = item(Region::Us, 1);
        assert_eq!(with_link.key(), "US1");
        let without = NewsItem { link: String::new(), title: "T".into(), ..with_link };
        assert_eq!(without.key(), "T");
    }

    #[test]
    fn a_language_pair_is_chosen_only_for_news_not_in_the_language_of_the_screens() {
        assert_eq!(language_pair(Region::Us, true), Some("en|de"));
        assert_eq!(language_pair(Region::Uk, true), Some("en|de"));
        assert_eq!(language_pair(Region::De, true), None);
        assert_eq!(language_pair(Region::De, false), Some("de|en"));
        assert_eq!(language_pair(Region::Us, false), None);
    }

    #[test]
    fn a_long_text_is_cut_at_sentence_ends_into_pieces_the_service_takes() {
        assert_eq!(pieces("One. Two! Three?", 100), ["One. Two! Three?"]);
        assert_eq!(pieces("One sentence. Another sentence. And a third one.", 30), ["One sentence.", "Another sentence.", "And a third one."]);
        let long = "word ".repeat(40);
        let parts = pieces(&long, 50);
        assert!(parts.len() >= 4 && parts.iter().all(|p| p.chars().count() <= 50), "{parts:?}");
        assert_eq!(pieces("3.5 percent rose", 100), ["3.5 percent rose"], "a point inside a number is no sentence end");
        assert_eq!(pieces("   ", 10), Vec::<String>::new());
    }

    #[test]
    fn the_answer_of_the_service_gives_the_text_or_the_reason_it_gave_none() {
        let ok = serde_json::json!({ "responseStatus": 200, "responseData": { "translatedText": "Die Fed h&#39;lt die Zinsen" } });
        assert_eq!(parse_translation(&ok).unwrap(), "Die Fed h'lt die Zinsen");
        let as_text = serde_json::json!({ "responseStatus": "200", "responseData": { "translatedText": "Hallo" } });
        assert_eq!(parse_translation(&as_text).unwrap(), "Hallo");
        let limit = serde_json::json!({ "responseStatus": 200, "responseData": { "translatedText": "MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY." } });
        assert!(parse_translation(&limit).unwrap_err().contains("daily limit"));
        let busy = serde_json::json!({ "responseStatus": 429, "responseDetails": "too many requests", "responseData": { "translatedText": "" } });
        assert_eq!(parse_translation(&busy).unwrap_err(), "the service answered 429 too many requests");
        assert!(parse_translation(&serde_json::json!({ "responseStatus": 200, "responseData": { "translatedText": " " } })).is_err());
    }
}
