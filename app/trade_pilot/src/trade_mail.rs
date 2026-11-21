//! The texts of the trade notifications (opened, stopped, closed): an HTML body for the email with the
//! trade and the table of today's trades, and a short plain text for the webhook. Plain logic, no `App`.

use crate::storage::StoredTrade;
use rust_decimal::Decimal;

/// One notification, ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeMail {
    /// The subject line of the email; also the event name of the webhook.
    pub subject: String,
    /// The body of the email, HTML.
    pub html: String,
    /// The same news as plain text (webhook, log).
    pub text: String,
}

/// Text that is put into the HTML body: `&`, `<` and `>` made harmless.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// "2 SPX 6650/6640 P PutSpread": what the trade is.
fn describe(trade: &StoredTrade) -> String {
    format!("{} {} {} {}", trade.qty, trade.symbol, trade.strikes, trade.trade_type)
}

/// The time of day of the opening (the stored text is "DD-MM-YYYY HH:MM:SS").
fn opened_time(trade: &StoredTrade) -> &str {
    trade.opened.split_whitespace().last().unwrap_or("")
}

/// The table of today's trades.
fn today_table(today: &[&StoredTrade], account: &dyn Fn(&str) -> String) -> String {
    let rows: String = today
        .iter()
        .map(|trade| {
            let stop = trade.stop_price.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
            let close = trade.price_close.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
            let cells = [
                escape(opened_time(trade)),
                escape(&account(&trade.account)),
                trade.qty.to_string(),
                escape(&trade.trade_type),
                escape(&trade.symbol),
                escape(&trade.strikes),
                trade.price_open.normalize().to_string(),
                stop,
                close,
                escape(&trade.status),
            ];
            ["<tr><td>", &cells.join("</td><td>"), "</td></tr>"].concat()
        })
        .collect();
    format!(
        "<br><b>Trades today:</b><br><table cellpadding=\"4\"><tr><td>Time Opened</td><td>Account</td><td>Qty</td><td>Trade</td><td>Symbol</td>\
         <td>Strikes</td><td>Open Price</td><td>Stop Target</td><td>Close Price</td><td>Status</td></tr>{rows}</table>"
    )
}

/// The mail about one or more trades: the heading lines (HTML-safe text), the account the subject names, and
/// the table of today's trades below.
fn mail(title: &str, account: &str, lines: &[String], today: &[&StoredTrade], mask: &dyn Fn(&str) -> String) -> TradeMail {
    let heading: String = std::iter::once(title).chain(lines.iter().map(String::as_str)).map(|line| escape(line) + "<br>").collect();
    let text = std::iter::once(title).chain(lines.iter().map(String::as_str)).collect::<Vec<_>>().join("\n");
    let html = format!("<html><body>{heading}{}</body></html>", today_table(today, mask));
    TradeMail { subject: format!("{title} ({})", mask(account)), html, text }
}

/// "Entry filled": the trades the fill created (a put and a call side of one entry) and today's trades.
#[must_use]
pub fn trade_opened(opened: &[&StoredTrade], today: &[&StoredTrade], mask: &dyn Fn(&str) -> String) -> TradeMail {
    let account = opened.first().map_or("", |trade| trade.account.as_str());
    let lines: Vec<String> = opened
        .iter()
        .flat_map(|trade| {
            let stop = trade.stop_price.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
            [format!("{} @ {}", describe(trade), trade.price_open.normalize()), format!("Stop Target: {stop}")]
        })
        .collect();
    mail("Entry filled", account, &lines, today, mask)
}

/// "Stop filled": the stop of `trade` filled at its closing price.
#[must_use]
pub fn trade_stopped(trade: &StoredTrade, today: &[&StoredTrade], mask: &dyn Fn(&str) -> String) -> TradeMail {
    let close = trade.price_close.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
    let lines = [format!("{}, Stop @ {close}", describe(trade)), format!("Entry price: {}", trade.price_open.normalize())];
    mail("Stop filled", &trade.account, &lines, today, mask)
}

/// "Trade closed": `trade` was closed at its closing price (profit target or a close order).
#[must_use]
pub fn trade_closed(trade: &StoredTrade, today: &[&StoredTrade], mask: &dyn Fn(&str) -> String) -> TradeMail {
    let close = trade.price_close.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
    let lines = [format!("{} @ {close}", describe(trade)), format!("Entry price: {}", trade.price_open.normalize())];
    mail("Trade closed", &trade.account, &lines, today, mask)
}

/// How far the price a credit trade was stopped at lies beyond its stop, in percent of the stop (`None` for a trade
/// without a stop, one that is not a credit trade, or one without a closing price). 0 is a fill at the stop.
#[must_use]
pub fn slippage_percent(trade: &StoredTrade) -> Option<Decimal> {
    let stop = trade.stop_now().filter(|stop| *stop > Decimal::ZERO)?;
    if trade.price_open <= Decimal::ZERO {
        return None;
    }
    let paid = Decimal::ZERO.checked_sub(trade.price_close?)?;
    paid.checked_sub(stop)?.checked_mul(Decimal::ONE_HUNDRED)?.checked_div(stop)
}

/// "ALERT: Stop filled with high slippage": the stop of `trade` filled `slippage` percent beyond its price.
#[must_use]
pub fn high_slippage(trade: &StoredTrade, slippage: Decimal, today: &[&StoredTrade], mask: &dyn Fn(&str) -> String) -> TradeMail {
    let stop = trade.stop_now().map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
    let close = trade.price_close.map_or_else(|| "-".to_string(), |price| price.normalize().to_string());
    let lines = [
        format!("{}, stop {stop}, filled at {close}", describe(trade)),
        format!("Slippage: {}%", slippage.round_dp(1).normalize()),
        format!("Entry price: {}", trade.price_open.normalize()),
        "A fill this far beyond the stop may be eligible for a trade bust or price adjustment under the exchange rules. Contact your broker without delay if you want to ask for one.".to_string(),
    ];
    let mut mail = mail("Stop filled with high slippage", &trade.account, &lines, today, mask);
    mail.subject = format!("ALERT: Stop filled with high slippage ({})", mask(&trade.account));
    mail
}

/// "ALERT: Trade without a stop": what is wrong, one line per trade.
#[must_use]
pub fn missing_stop(problems: &[String]) -> TradeMail {
    let mut text = String::from("Missing Stop(s) Detected");
    let mut html = format!("{}<br>", escape(&text));
    for problem in problems {
        text.push('\n');
        text.push_str(problem);
        html.push_str(&(escape(problem) + "<br>"));
    }
    TradeMail { subject: "ALERT: Trade without a stop".to_string(), html: format!("<html><body>{html}</body></html>"), text }
}

/// A warning mail: the `heading`, one line per entry of `lines`, and a closing `footer` (may be empty).
fn alert(subject: &str, heading: &str, lines: &[String], footer: &str) -> TradeMail {
    let mut parts: Vec<&str> = vec![heading];
    parts.extend(lines.iter().map(String::as_str));
    let mut html: String = parts.iter().map(|line| escape(line) + "<br>").collect();
    let mut text = parts.join(
        "
",
    );
    if !footer.is_empty() {
        html = [html.as_str(), "<br>", &escape(footer), "<br>"].concat();
        text = [
            text.as_str(),
            "

",
            footer,
        ]
        .concat();
    }
    TradeMail { subject: subject.to_string(), html: ["<html><body>", &html, "</body></html>"].concat(), text }
}

/// "Daily profit target reached": today's profit of `total` dollars reached the target.
#[must_use]
pub fn daily_profit_target(total: Decimal, target: Decimal) -> TradeMail {
    let lines = [format!("Profit today: {} (target {})", total.round_dp(2).normalize(), target.normalize())];
    alert(
        "Daily profit target reached",
        "Daily profit target reached",
        &lines,
        "Waiting scheduled entries were cancelled and no more are made today. TradePilot does not close the open trades: please review your account positions.",
    )
}

/// "ALERT: Exit order over-filled": an exit order filled although the trade was closed already.
#[must_use]
pub fn over_fill(problems: &[String]) -> TradeMail {
    alert("ALERT: Exit order over-filled", "Exit order over-filled", problems, "Check your positions in TWS: more contracts were closed than were open.")
}

/// "ALERT: Price beyond the stop": the market is beyond the stop price of an open trade.
#[must_use]
pub fn beyond_stop(problems: &[String]) -> TradeMail {
    alert(
        "ALERT: Price beyond the stop",
        "Price beyond the stop",
        problems,
        "The stop order has not filled although the price is beyond it. Check the order in TWS.",
    )
}

/// "ALERT: Short options expire today": a short option of an ETF or stock expires today.
#[must_use]
pub fn open_short(problems: &[String]) -> TradeMail {
    alert(
        "ALERT: Short options expire today",
        "Open Short 0 DTE ETF/Equity Position(s) Detected",
        problems,
        "Short options in these underlyings carry stock assignment risk, even if they are not currently in the money. Close these positions in your brokerage account before the close.",
    )
}

/// "ALERT: Open trade cannot be valued": an open trade cannot be valued.
#[must_use]
pub fn quotes_missing(problems: &[String]) -> TradeMail {
    alert(
        "ALERT: Open trade cannot be valued",
        "Trade Quotes Missing or Outdated",
        problems,
        "Without current quotes the exit conditions cannot be checked. Open the Quotes page and click Refresh Quote Subscriptions; if that does not help, restart TWS and reconnect.",
    )
}

/// "ALERT: Timed close not completed": the timed exit of a trade did not complete within its window.
#[must_use]
pub fn close_aborted(label: &str, planned: &str) -> TradeMail {
    alert(
        "ALERT: Timed close not completed",
        "Timed close not completed",
        &[format!("{label}: the exit planned for {planned} did not complete within 5 minutes.")],
        "Please review your account and close the trade yourself if necessary.",
    )
}

/// "ALERT: Order problem on an open trade": TWS refused or dropped an order of an open trade (a stop that cannot be placed, say).
#[must_use]
pub fn processing_error(problems: &[String]) -> TradeMail {
    alert(
        "ALERT: Order problem on an open trade",
        "An order of an open trade could not be processed.",
        problems,
        "Check the trade in TWS: it may be without a stop.",
    )
}

/// A margin rejection: of a scheduled entry (`entry` names it) or of any other order (`message` is TWS's text).
#[must_use]
pub fn margin_issue(entry: Option<&str>, account: &str, message: &str) -> TradeMail {
    match entry {
        Some(entry) => alert(
            "Scheduled Trade Entry Aborted - Margin Issue",
            "Scheduled Trade Entry Aborted - Margin Issue",
            &[format!("{entry} did not complete successfully due to a margin error. Account: {account}"), message.to_string()],
            "",
        ),
        None => alert(
            "Unexpected Order Error - Margin Issue",
            "Unexpected Order Error - Margin Issue",
            &["An order was rejected because of a margin error.".to_string(), format!("Account: {account}"), message.to_string()],
            "",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn trade() -> StoredTrade {
        let mut trade: StoredTrade = serde_json::from_str(
            r#"{"day":"2026-11-13","opened":"13-11-2026 09:35:02","account":"DU1234567","symbol":"SPX","trade_type":"PutSpread",
                "strikes":"6650/6640 P","order_id":10,"qty":2,"multiplier":100,"price_open":"0.30","delta":0.0}"#,
        )
        .unwrap();
        trade.stop_price = Some(Decimal::new(90, 2));
        trade
    }

    fn mask(account: &str) -> String {
        format!("masked-{}", account.len())
    }

    #[test]
    fn an_opened_trade_names_the_trade_its_stop_and_the_table_of_today() {
        let trade = trade();
        let mail = trade_opened(&[&trade], &[&trade], &mask);
        assert_eq!(mail.subject, "Entry filled (masked-9)");
        assert!(mail.html.contains("2 SPX 6650/6640 P PutSpread @ 0.3<br>Stop Target: 0.9<br>"), "{}", mail.html);
        assert!(mail.html.contains("<td>09:35:02</td><td>masked-9</td><td>2</td><td>PutSpread</td>"), "{}", mail.html);
        assert!(mail.text.starts_with("Entry filled\n2 SPX 6650/6640 P PutSpread @ 0.3\nStop Target: 0.9"), "{}", mail.text);
        assert!(!mail.html.contains("DU1234567"), "the account stays masked");
    }

    #[test]
    fn a_stopped_and_a_closed_trade_show_their_closing_price() {
        let mut trade = trade();
        trade.price_close = Some(Decimal::new(90, 2));
        trade.status = "Stopped".to_string();
        let stopped = trade_stopped(&trade, &[&trade], &mask);
        assert_eq!(stopped.subject, "Stop filled (masked-9)");
        assert!(stopped.html.contains("2 SPX 6650/6640 P PutSpread, Stop @ 0.9<br>Entry price: 0.3<br>"), "{}", stopped.html);
        assert!(stopped.html.contains("<td>0.9</td><td>Stopped</td>"), "{}", stopped.html);
        let closed = trade_closed(&trade, &[], &mask);
        assert!(closed.html.contains("2 SPX 6650/6640 P PutSpread @ 0.9<br>"), "{}", closed.html);
        assert_eq!(closed.text, "Trade closed\n2 SPX 6650/6640 P PutSpread @ 0.9\nEntry price: 0.3");
    }

    #[test]
    fn slippage_is_the_percent_a_credit_trade_was_stopped_beyond_its_stop() {
        let mut trade = trade();
        trade.price_close = Some(Decimal::new(-90, 2));
        assert_eq!(slippage_percent(&trade), Some(Decimal::ZERO), "a fill at the stop of 0.90");
        trade.price_close = Some(Decimal::new(-180, 2));
        assert_eq!(slippage_percent(&trade), Some(Decimal::ONE_HUNDRED), "paid twice the stop");
        trade.stop_price = None;
        assert_eq!(slippage_percent(&trade), None, "no stop, no slippage");
        let mail = high_slippage(&trade_with_stop(), Decimal::ONE_HUNDRED, &[], &mask);
        assert_eq!(mail.subject, "ALERT: Stop filled with high slippage (masked-9)");
        assert!(mail.html.contains("Slippage: 100%"), "{}", mail.html);
        let missing = missing_stop(&["SPX PutSpread: no stop could be placed".to_string()]);
        assert_eq!(missing.subject, "ALERT: Trade without a stop");
        assert_eq!(missing.text, "Missing Stop(s) Detected\nSPX PutSpread: no stop could be placed");
    }

    fn trade_with_stop() -> StoredTrade {
        let mut trade = trade();
        trade.price_close = Some(Decimal::new(-180, 2));
        trade
    }

    #[test]
    fn the_warning_mails_name_the_problem_and_what_to_do() {
        let target = daily_profit_target(Decimal::new(120_050, 2), Decimal::new(1000, 0));
        assert_eq!(target.subject, "Daily profit target reached");
        assert!(target.text.contains("Profit today: 1200.5 (target 1000)"), "{}", target.text);
        let short = open_short(&["IWM 240 P 20261117 (Position: -1)".to_string()]);
        assert_eq!(short.subject, "ALERT: Short options expire today");
        assert!(short.html.contains("IWM 240 P 20261117 (Position: -1)<br>") && short.html.contains("assignment risk"), "{}", short.html);
        assert_eq!(close_aborted("SPX PutSpread 7300/7290 P", "11:30").subject, "ALERT: Timed close not completed");
        assert_eq!(over_fill(&[String::from("x")]).subject, "ALERT: Exit order over-filled");
        assert_eq!(beyond_stop(&[String::from("x")]).subject, "ALERT: Price beyond the stop");
        assert_eq!(quotes_missing(&[String::from("x")]).subject, "ALERT: Open trade cannot be valued");
        assert_eq!(processing_error(&[String::from("x")]).subject, "ALERT: Order problem on an open trade");
        let scheduled = margin_issue(Some("Schedule run 3"), "DU1", "TWS 201: margin");
        assert_eq!(scheduled.subject, "Scheduled Trade Entry Aborted - Margin Issue");
        assert_eq!(margin_issue(None, "DU1", "m").subject, "Unexpected Order Error - Margin Issue");
    }

    #[test]
    fn text_that_could_be_markup_is_made_harmless() {
        let mut trade = trade();
        trade.symbol = "A<B>&".to_string();
        let mail = trade_opened(&[&trade], &[&trade], &mask);
        assert!(mail.html.contains("A&lt;B&gt;&amp;") && !mail.html.contains("A<B>"), "{}", mail.html);
    }
}
