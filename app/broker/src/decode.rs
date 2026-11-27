//! Decoders: what a message from TWS says, as events and values.

use crate::{
    ContractData, ExecutionRow, OpenOrderInfo, OpenOrderLeg, PortfolioRow, Position, TwsEvent, IB_ACCOUNT_SUMMARY_REQUEST_ID, IN_MARKET_DATA_TYPE,
    IN_TICK_GENERIC, IN_TICK_REQ_PARAMS, IN_TICK_STRING,
};

/// IB's documented convention for paper-trading account identifiers (e.g. "DU1234567"), used only
/// as a safety heuristic before any order is sent. This is IB's own account-numbering convention,
/// not something TWS itself checks.
/// It is combined with a mandatory manual confirmation in the UI; neither check
/// alone is trusted.
#[must_use]
pub fn looks_like_paper_account(account_id: &str) -> bool {
    account_id.trim().to_ascii_uppercase().starts_with("DU")
}

/// Masks an account number for the log and mails: first 4 + "***" + last 4 characters (8+ long only). The input may look like
/// "IB:DU1234567", so "IB:" counts towards the first 4.
#[must_use]
pub fn mask_account(account: &str) -> String {
    let len = account.chars().count();
    if len < 8 {
        return account.to_string();
    }
    let head: String = account.chars().take(4).collect();
    let tail: String = account.chars().skip(len - 4).collect();
    format!("{head}***{tail}")
}

/// `IBApi.EDecoder.HistoricalDataUpdateEvent`: reqId, bar count, then date, open, close, high, low, wap, volume of the
/// bar still forming. Yields its close as a bar of the request.
#[must_use]
pub fn parse_historical_update(fields: &[String]) -> Option<TwsEvent> {
    let req_id: i32 = fields.get(1)?.parse().ok()?;
    let date = fields.get(3)?.clone();
    let close: f64 = fields.get(5)?.parse().ok()?;
    let high: f64 = fields.get(6).and_then(|text| text.parse().ok()).unwrap_or(close);
    let low: f64 = fields.get(7).and_then(|text| text.parse().ok()).unwrap_or(close);
    Some(TwsEvent::HistoricalBar { req_id, date, high, low, close })
}

/// `IBApi.EDecoder.HistoricalDataEvent` (server version 196+, no leading version, no start/end
/// strings): reqId, bar count, then per bar time, open, high, low, close, volume, wap, count.
/// Yields the date and close of every bar; the last one is the latest.
#[must_use]
pub fn parse_historical_bars(fields: &[String]) -> Option<Vec<TwsEvent>> {
    let req_id: i32 = fields.get(1)?.parse().ok()?;
    let count: usize = fields.get(2)?.parse().ok()?;
    let mut events = Vec::new();
    for i in 0..count {
        let base = 3usize.checked_add(i.checked_mul(8)?)?;
        let date = fields.get(base)?.clone();
        let close: f64 = fields.get(base.checked_add(4)?)?.parse().ok()?;
        let high: f64 = fields.get(base.checked_add(2)?).and_then(|text| text.parse().ok()).unwrap_or(close);
        let low: f64 = fields.get(base.checked_add(3)?).and_then(|text| text.parse().ok()).unwrap_or(close);
        events.push(TwsEvent::HistoricalBar { req_id, date, high, low, close });
    }
    Some(events)
}

/// The order id of an `openOrder` message (`IBApi.EDecoder.OpenOrderEvent`): below server version
/// 145 (`ORDER_CONTAINER`) a version field comes first, from 145 on the order id directly.
#[must_use]
pub fn parse_open_order_id(fields: &[String], server_version: i32) -> Option<i32> {
    fields.get(open_order_first_field(server_version))?.parse().ok()
}

/// The place of the order id in an `openOrder`: after the message version below server version 145, else right after the message id.
#[must_use]
pub fn open_order_first_field(server_version: i32) -> usize {
    if server_version < 145 {
        2
    } else {
        1
    }
}

/// The account of an `openOrder`, counted from its order id (see `parse_open_order`).
pub const OPEN_ORDER_ACCOUNT: usize = 19;

/// The permId of an `openOrder`, counted from its order id: the last field `parse_open_order` reads.
pub const OPEN_ORDER_PERM_ID: usize = 24;

/// The account of an `updatePortfolio`, its last field (see `parse_portfolio_value`).
pub const PORTFOLIO_ACCOUNT: usize = 19;

/// The message version of an `openOrder` from which every contract field this reads is sent (multiplier and trading class).
const OPEN_ORDER_FULL_CONTRACT: i32 = 32;

/// The head of an `openOrder` message (`IBApi.EDecoder.OpenOrderEvent` with its `OrderDecoder`): the order id, the contract (conId,
/// symbol, secType, expiry, strike, right, multiplier, exchange, currency, localSymbol, tradingClass), then action, totalQuantity,
/// orderType, lmtPrice, auxPrice, tif, ocaGroup, account, openClose, origin, orderRef, clientId (message version 3 on) and permId
/// (version 4 on). These come first in every version this app accepts (the message version is the server version from 145 on, else
/// its own field, 32 or more for the full contract); what follows them (algo parameters, combo legs, conditions, the order state)
/// depends on the version and is not read. `None` when the head is missing or a number in it cannot be read.
#[must_use]
pub fn parse_open_order(fields: &[String], server_version: i32) -> Option<OpenOrderInfo> {
    let (version, first): (i32, usize) = if server_version < 145 { (fields.get(1)?.trim().parse::<i32>().ok()?, 2) } else { (server_version, 1) };
    if version < OPEN_ORDER_FULL_CONTRACT {
        return None;
    }
    let text = |offset: usize| -> Option<String> { fields.get(first.checked_add(offset)?).map(|f| f.trim().to_string()) };
    // TWS sends an unset price as an empty field or as the largest double.
    let price = |offset: usize| -> Option<Option<f64>> {
        let value = text(offset)?;
        if value.is_empty() {
            return Some(None);
        }
        let number: f64 = value.parse().ok()?;
        Some((number.is_finite() && number.abs() < 1.0e300).then_some(number))
    };
    let number = |offset: usize| -> Option<f64> {
        let value = text(offset)?;
        if value.is_empty() {
            Some(0.0)
        } else {
            value.parse().ok().filter(|n: &f64| n.is_finite())
        }
    };
    Some(OpenOrderInfo {
        order_id: text(0)?.parse().ok()?,
        con_id: text(1)?.parse().ok()?,
        symbol: text(2)?,
        sec_type: text(3)?,
        expiry: text(4)?,
        strike: number(5)?,
        right: text(6)?,
        // 7 multiplier, 8 exchange, 9 currency, 10 localSymbol, 11 tradingClass
        action: text(12)?,
        total_quantity: number(13)?,
        order_type: text(14)?,
        limit_price: price(15)?,
        aux_price: price(16)?,
        tif: text(17)?,
        oca_group: text(18)?,
        account: text(OPEN_ORDER_ACCOUNT)?,
        // 20 openClose, 21 origin
        order_ref: text(22)?,
        client_id: text(23)?.parse().ok()?,
        perm_id: text(OPEN_ORDER_PERM_ID)?.parse().ok()?,
        combo_legs: open_order_combo_legs(fields, server_version).map(|(legs, _)| legs),
        trail_stop_price: open_order_trail_stop_price(fields, server_version),
    })
}

/// The combo legs of an `openOrder` and the place of the first field after them. From the permId on the message is walked as
/// `IBApi.EOrderDecoder` reads it for message version 32 and up (`OpenOrderEvent`): outsideRth, hidden, discretionaryAmt,
/// goodAfterTime, sharesAllocation, faGroup, faMethod, faPercentage (one more field below server version 177), modelCode (server
/// version 103 on), goodTillDate, rule80A, percentOffset, settlingFirm, shortSaleSlot, designatedLocation, exemptCode,
/// auctionStrategy, startingPrice, stockRefPrice, delta, stockRangeLower, stockRangeUpper, displaySize, blockOrder, sweepToFill,
/// allOrNone, minQty, ocaType, eTradeOnly, firmQuoteOnly, nbboPriceCap, parentId, triggerMethod, volatility, volatilityType,
/// deltaNeutralOrderType, deltaNeutralAuxPrice (with a delta-neutral order type eight more), continuousUpdate,
/// referencePriceType, trailStopPrice, trailingPercent, basisPoints, basisPointsType, comboLegsDescription, then the number of
/// legs and for each conId, ratio, action, exchange, openClose, shortSaleSlot, designatedLocation, exemptCode, then the number of
/// order combo legs and a price for each. An order of one contract has no legs. `None` when the message ends early or a count
/// or leg cannot be read.
#[must_use]
pub fn open_order_combo_legs(fields: &[String], server_version: i32) -> Option<(Vec<OpenOrderLeg>, usize)> {
    let field = |index: usize| fields.get(index).map(|f| f.trim());
    // trailStopPrice, trailingPercent, the basis points fields, comboLegsDescription.
    let mut index = open_order_trail_stop_field(fields, server_version)?.checked_add(5)?;
    let count: usize = field(index)?.parse().ok()?;
    index = index.checked_add(1)?;
    let mut legs = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        legs.push(OpenOrderLeg {
            con_id: field(index)?.parse().ok()?,
            ratio: field(index.checked_add(1)?)?.parse().ok()?,
            action: field(index.checked_add(2)?)?.to_string(),
        });
        index = index.checked_add(8)?;
    }
    let prices: usize = field(index)?.parse().ok()?;
    let after = index.checked_add(1)?.checked_add(prices)?;
    (after <= fields.len()).then_some((legs, after))
}

/// The place of the trailStopPrice of an `openOrder` (walked as in `open_order_combo_legs`): after deltaNeutralOrderType, its aux
/// price (with a delta-neutral order type eight more fields), continuousUpdate and referencePriceType. `None` when the message ends
/// before deltaNeutralOrderType.
fn open_order_trail_stop_field(fields: &[String], server_version: i32) -> Option<usize> {
    let first = open_order_first_field(server_version);
    // From outsideRth to faPercentage, then the optional fields that depend on the server version.
    let mut index = first.checked_add(OPEN_ORDER_PERM_ID)?.checked_add(9)?;
    if server_version < 177 {
        index = index.checked_add(1)?;
    }
    if server_version >= 103 {
        index = index.checked_add(1)?;
    }
    // goodTillDate up to deltaNeutralOrderType.
    index = index.checked_add(26)?;
    let delta_neutral = !fields.get(index)?.trim().is_empty();
    index.checked_add(if delta_neutral { 12 } else { 4 })
}

/// The trailStopPrice of an `openOrder` (`IBApi.Order.TrailStopPrice`, read by `EOrderDecoder.readTrailParams`): the stop price of a
/// trailing stop. `None` when TWS sends none (an empty field or its "unset" value, the largest double) or the field cannot be read.
#[must_use]
pub fn open_order_trail_stop_price(fields: &[String], server_version: i32) -> Option<f64> {
    let value = fields.get(open_order_trail_stop_field(fields, server_version)?)?.trim();
    value.parse::<f64>().ok().filter(|number| number.is_finite() && number.abs() < 1.0e300)
}

/// The `clientId` of an `orderStatus` message (`IBApi.EDecoder.OrderStatusEvent`): from server version 131 on, after orderId, status,
/// filled, remaining, avgFillPrice, permId, parentId and lastFillPrice. `None` for an older server or a field that cannot be read.
#[must_use]
pub fn order_status_client_id(fields: &[String], server_version: i32) -> Option<i32> {
    if server_version < 131 {
        return None;
    }
    fields.get(9)?.trim().parse().ok()
}

/// The `permId` of an `orderStatus` message, the field after avgFillPrice from server version 131 on (see `order_status_client_id`).
#[must_use]
pub fn order_status_perm_id(fields: &[String], server_version: i32) -> Option<i64> {
    if server_version < 131 {
        return None;
    }
    fields.get(6)?.trim().parse().ok()
}

/// `IBApi.EDecoder.TickPriceEvent`'s real field order: version, tickerId, tickType, price,
/// [v>=2: size], [v>=3: attribute bitmask]. This app only uses the price; the bundled size
/// (relevant to bid/ask/last) is not yet surfaced - a documented scope limit, not a guess.
#[must_use]
pub fn parse_tick_price(fields: &[String]) -> Option<TwsEvent> {
    let ticker_id: i32 = fields.get(2)?.parse().ok()?;
    let tick_type: i32 = fields.get(3)?.parse().ok()?;
    let price: f64 = fields.get(4)?.parse().ok()?;
    Some(TwsEvent::TickPrice { ticker_id, tick_type, price })
}

/// The commission of a `commissionReport` message (`59|version|execId|commission|currency|...`): `None` when the
/// execution id is missing or TWS sends no amount (its "unset" value is the largest double).
#[must_use]
pub fn parse_commission_report(fields: &[String]) -> Option<TwsEvent> {
    let exec_id = fields.get(2).map(|id| id.trim()).filter(|id| !id.is_empty())?;
    let commission: f64 = fields.get(3)?.trim().parse().ok().filter(|c: &f64| c.is_finite() && c.abs() < 1.0e300)?;
    let currency = fields.get(4).map_or_else(String::new, |c| c.trim().to_string());
    Some(TwsEvent::Commission { exec_id: exec_id.to_string(), commission, currency })
}

/// The accounts of a `managedAccounts` message (empty for none).
#[must_use]
pub fn parse_managed_accounts(fields: &[String]) -> Vec<String> {
    fields.get(2).map(|list| list.split(',').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

/// The ticker id of a price-less reqMktData reply (tickReqParams / marketDataType / tickString /
/// tickGeneric), i.e. proof that TWS accepted that subscription. `None` for any other message.
#[must_use]
pub fn parse_market_data_ack(fields: &[String]) -> Option<i32> {
    let message_type: i32 = fields.first()?.parse().ok()?;
    let ticker_index = match message_type {
        IN_TICK_REQ_PARAMS => 1,
        IN_TICK_GENERIC | IN_TICK_STRING | IN_MARKET_DATA_TYPE => 2,
        _ => return None,
    };
    fields.get(ticker_index)?.parse().ok()
}

/// `IBApi.EDecoder.TickSizeEvent`'s real field order: version, tickerId, field, size.
#[must_use]
pub fn parse_tick_size(fields: &[String]) -> Option<TwsEvent> {
    let ticker_id: i32 = fields.get(2)?.parse().ok()?;
    let field: i32 = fields.get(3)?.parse().ok()?;
    let size: f64 = fields.get(4)?.parse().ok()?;
    Some(TwsEvent::TickSize { ticker_id, field, size })
}

/// `IBApi.EDecoder.OrderStatusEvent`'s real field order (server version >= 131 has no leading
/// version field at all; this app only negotiates versions in that range).
#[must_use]
pub fn parse_order_status(fields: &[String], server_version: i32) -> Option<TwsEvent> {
    let has_version_field = server_version < 131;
    let mut idx = 1usize;
    if has_version_field {
        idx += 1;
    }
    let order_id: i32 = fields.get(idx)?.parse().ok()?;
    idx += 1;
    let status = fields.get(idx)?.clone();
    idx += 1;
    let filled: f64 = fields.get(idx)?.parse().ok()?;
    idx += 1;
    let remaining: f64 = fields.get(idx)?.parse().ok()?;
    idx += 1;
    // `avgFillPrice` (ReadDouble): 0 until something filled; missing only in a truncated message.
    let avg_fill_price: f64 = fields.get(idx).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    Some(TwsEvent::OrderStatus { order_id, status, filled, remaining, avg_fill_price })
}

/// `IBApi.EDecoder.ContractDataEvent`'s real field order, up to `Contract.ConId`; later fields are
/// not needed by this app.
#[must_use]
pub fn parse_contract_data(fields: &[String], server_version: i32) -> Option<ContractData> {
    fn take<'a>(fields: &'a [String], i: &mut usize) -> Option<&'a str> {
        let value = fields.get(*i).map(String::as_str);
        *i += 1;
        value
    }
    let mut i = 1usize; // fields[0] is the message id ("10")
    let num = if server_version < 164 { take(fields, &mut i)?.parse::<i32>().ok()? } else { 8 };
    if num < 3 {
        return None; // reqId isn't present in this message shape; not expected from this app
    }
    let req_id: i32 = take(fields, &mut i)?.parse().ok()?;
    let symbol = take(fields, &mut i)?.to_string();
    let sec_type = take(fields, &mut i)?.to_string();
    let last_trade = take(fields, &mut i)?; // lastTradeDateOrContractMonth ("yyyyMMdd[ hh:mm...]")
    let expiry = last_trade.get(..8).unwrap_or(last_trade).to_string();
    if server_version >= 182 {
        take(fields, &mut i)?; // lastTradeDate (separate field only from server_version 182)
    }
    let strike: f64 = take(fields, &mut i)?.parse().unwrap_or(0.0);
    let right = take(fields, &mut i)?.to_string();
    let exchange = take(fields, &mut i)?.to_string();
    let currency = take(fields, &mut i)?.to_string();
    let local_symbol = take(fields, &mut i)?.to_string();
    take(fields, &mut i)?; // marketName
    let trading_class = take(fields, &mut i)?.to_string();
    let con_id: i32 = take(fields, &mut i)?.parse().ok()?;
    Some(ContractData { req_id, con_id, symbol, sec_type, expiry, strike, right, exchange, currency, local_symbol, trading_class })
}

/// `IBApi.EDecoder.TickOptionComputationEvent`: ticker id, tick type and the option's delta.
/// The leading version field is gone from server version 156 on, where a `tickAttrib` follows
/// the tick type instead. TWS sends -2 for "delta not computed", reported here as `None`.
#[must_use]
pub fn parse_option_delta(fields: &[String], server_version: i32) -> Option<(i32, i32, Option<f64>)> {
    let (ticker_index, delta_index) = if server_version >= 156 { (1, 5) } else { (2, 5) };
    let ticker_id: i32 = fields.get(ticker_index)?.parse().ok()?;
    let tick_type: i32 = fields.get(ticker_index + 1)?.parse().ok()?;
    let delta = fields.get(delta_index)?.parse::<f64>().ok().filter(|d| d.total_cmp(&-2.0).is_ne() && d.is_finite());
    Some((ticker_id, tick_type, delta))
}

/// `IBApi.EDecoder.SecurityDefinitionOptionParameterEvent`'s real field order: reqId, exchange,
/// underlyingConId, tradingClass,
/// multiplier, expirationCount, expirations[expirationCount], strikeCount, strikes[strikeCount].
/// No leading "version" field, consistent with the outgoing request it answers.
#[must_use]
pub fn parse_security_definition_option_parameter(fields: &[String]) -> Option<TwsEvent> {
    let mut i = 1usize; // fields[0] is the message id ("75")
    let req_id: i32 = fields.get(i)?.parse().ok()?;
    i += 1;
    let exchange = fields.get(i)?.clone();
    i += 1;
    let underlying_con_id: i32 = fields.get(i)?.parse().ok()?;
    i += 1;
    let trading_class = fields.get(i)?.clone();
    i += 1;
    let multiplier = fields.get(i)?.clone();
    i += 1;
    let expiration_count: usize = fields.get(i)?.parse().ok()?;
    i += 1;
    let mut expirations = Vec::with_capacity(expiration_count);
    for _ in 0..expiration_count {
        expirations.push(fields.get(i)?.clone());
        i += 1;
    }
    let strike_count: usize = fields.get(i)?.parse().ok()?;
    i += 1;
    let mut strikes = Vec::with_capacity(strike_count);
    for _ in 0..strike_count {
        strikes.push(fields.get(i)?.parse().ok()?);
        i += 1;
    }
    Some(TwsEvent::OptionParams { req_id, exchange, underlying_con_id, trading_class, multiplier, expirations, strikes })
}

/// Account, tag, value and currency of an `accountSummary` message for `TradePilot`'s own request (`None` for another request); an error for a message that stops short.
///
/// # Errors
///
/// The reason as text when the message stops short or a field is invalid.
pub fn parse_account_summary(fields: &[String]) -> Result<Option<(String, String, String, String)>, String> {
    if fields.get(2).and_then(|id| id.parse::<i32>().ok()) != Some(IB_ACCOUNT_SUMMARY_REQUEST_ID) {
        return Ok(None);
    }
    match fields.get(3..7) {
        Some([account, tag, value, currency]) => Ok(Some((account.clone(), tag.clone(), value.clone(), currency.clone()))),
        _ => Err("Malformed TWS account summary message".into()),
    }
}

/// The message version of `updatePortfolio` from which all of its fields are sent (conId, multiplier and primary exchange, local
/// symbol, trading class, the cost and the profits, the account).
const PORTFOLIO_FULL_VERSION: i32 = 8;

/// An `updatePortfolio` message (`IBApi.EDecoder.PortfolioValueEvent`): version, conId, the contract (symbol, secType, expiry,
/// strike, right, multiplier, primary exchange, currency, local symbol, trading class), then position, market price, market
/// value, average cost, unrealized and realized profit, account. `None` for an older version, which leaves fields out, or for a
/// field that cannot be read.
#[must_use]
pub fn parse_portfolio_value(fields: &[String]) -> Option<PortfolioRow> {
    let version: i32 = fields.get(1)?.trim().parse().ok()?;
    if version < PORTFOLIO_FULL_VERSION {
        return None;
    }
    let number = |index: usize| -> Option<f64> { fields.get(index)?.trim().parse::<f64>().ok().filter(|n| n.is_finite()) };
    Some(PortfolioRow {
        con_id: fields.get(2)?.trim().parse().ok()?,
        quantity: number(13)?,
        market_price: number(14)?,
        market_value: number(15)?,
        average_cost: number(16)?,
        unrealized_pnl: number(17)?,
        account: fields.get(PORTFOLIO_ACCOUNT)?.trim().to_string(),
    })
}

/// A `position` message; an error for a missing field or a quantity or cost that is no finite number.
///
/// # Errors
///
/// The reason as text for a missing field or a quantity or cost that is no finite number.
pub fn parse_position(fields: &[String]) -> Result<Option<Position>, String> {
    let version = fields.get(1).and_then(|v| v.parse::<u32>().ok()).filter(|v| *v >= 1).ok_or_else(|| "Invalid version in TWS position message".to_string())?;
    let quantity_index = if version >= 2 { 14 } else { 13 };
    let number = |index: usize, name: &str| -> Result<f64, String> {
        fields
            .get(index)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("Invalid {name} in TWS position message"))
    };
    let quantity = number(quantity_index, "quantity")?;
    let average_cost = if version >= 3 { Some(number(quantity_index + 1, "average cost")?) } else { None };
    let Some([account, contract_id, symbol, security_type]) = fields.get(2..6) else {
        return Err("Malformed TWS position message".to_string());
    };
    // Then the rest of the contract: expiry, strike, right, multiplier (empty for a stock).
    let text = |index: usize| fields.get(index).map_or_else(String::new, |value| value.trim().to_string());
    let strike = text(7).parse::<f64>().ok().filter(|value| value.is_finite()).unwrap_or(0.0);
    let multiplier = text(9).parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.0);
    Ok(Some(Position {
        account: account.clone(),
        contract_id: contract_id.clone(),
        symbol: symbol.clone(),
        security_type: security_type.clone(),
        expiry: text(6),
        strike,
        right: text(8),
        multiplier,
        quantity,
        average_cost,
    }))
}

/// `IBApi.EDecoder.ErrorEvent`'s real field order: [v<194: version], id, errorCode, errorMsg,
/// [v>=166: advancedOrderRejectJson], [v>=194: errorTime]. The leading version field disappears
/// entirely at server version 194+ (this app negotiates 200), so it must not be assumed present.
/// `id` is TWS's own `reqId` echo (e.g. the `reqMktData` ticker id this error is about, or -1 for
/// a connection-wide notice), kept so callers can tell which in-flight request an error belongs
/// to instead of only logging the code/text.
///
/// # Errors
///
/// The reason as text when the message stops short or a field is invalid.
pub fn tws_error_message(fields: &[String], server_version: i32) -> Result<(i32, i32, String), String> {
    let id_index = if server_version < 194 { 2 } else { 1 };
    let code_index = id_index + 1;
    let text_index = code_index + 1;
    let id = fields.get(id_index).and_then(|s| s.parse().ok()).ok_or_else(|| "Malformed TWS error message: missing request id".to_string())?;
    let code = fields.get(code_index).and_then(|s| s.parse().ok()).ok_or_else(|| "Malformed TWS error message: missing error code".to_string())?;
    let text = fields.get(text_index).filter(|s| !s.is_empty()).ok_or_else(|| format!("Malformed TWS error message {code}: missing description"))?;
    Ok((id, code, english_tws_message(code, &decode_unicode_escapes(text))))
}

/// The English text of a TWS message: TWS sends its texts in the language of the TWS installation
/// (e.g. "Verbindung zum Marktdatenzentrum ist OK:usfarm"), but the log is English. For the codes
/// below the text is IB's English one; what TWS puts after the last ':' (a farm name, a ticker id,
/// an order's reason) is kept as sent. Any other code keeps TWS's own text: its English wording is
/// not known here, and a guessed one would be worse than the original.
#[must_use]
pub fn english_tws_message(code: i32, text: &str) -> String {
    let with_tail = |english: &str| match text.rsplit_once(':') {
        Some((_, tail)) => format!("{english}:{}", tail.trim()),
        None => english.to_string(),
    };
    match code {
        2103 => with_tail("Market data farm connection is broken"),
        2104 => with_tail("Market data farm connection is OK"),
        2105 => with_tail("HMDS data farm connection is broken"),
        2106 => with_tail("HMDS data farm connection is OK"),
        2107 => with_tail("HMDS data farm connection is inactive but should be available upon demand"),
        2108 => with_tail("Market data farm connection is inactive but should be available upon demand"),
        2119 => with_tail("Market data farm is connecting"),
        2157 => with_tail("Sec-def data farm connection is broken"),
        2158 => with_tail("Sec-def data farm connection is OK"),
        300 => with_tail("Can't find EId with tickerId"),
        201 => with_tail("Order rejected - reason"),
        202 => with_tail("Order cancelled - reason"),
        1100 => "Connectivity between IB and Trader Workstation has been lost.".to_string(),
        1101 => "Connectivity between IB and Trader Workstation has been restored - data lost.".to_string(),
        1102 => "Connectivity between IB and Trader Workstation has been restored - data maintained.".to_string(),
        1300 => "TWS socket port has been reset and this connection is being dropped. Please reconnect on the new port.".to_string(),
        2100 => "API client has been unsubscribed from account data.".to_string(),
        2110 => "Connectivity between Trader Workstation and server is broken. It will be restored automatically.".to_string(),
        2151 => "Position information is not available yet.".to_string(),
        200 => "No security definition has been found for the request.".to_string(),
        326 => "Unable to connect as the client id is already in use. Retry with a unique client id.".to_string(),
        354 => "Requested market data is not subscribed.".to_string(),
        502 => "Couldn't connect to TWS. Confirm that the API is enabled in TWS (Configure > API).".to_string(),
        504 => "Not connected.".to_string(),
        10090 => "Part of requested market data is not subscribed.".to_string(),
        10167 => "Requested market data is not subscribed. Displaying delayed market data.".to_string(),
        10197 => "No market data during competing live session.".to_string(),
        _ => text.to_string(),
    }
}

/// What a code TWS sends in an error message means for the session. Source: IB's "Message Codes" page
/// (<https://interactivebrokers.github.io/tws-api/message_codes.html>): the tables "System Message Codes", "Warning Message Codes",
/// "Client Error Codes" and "TWS Error Codes". Codes not listed there are `Unclassified` and only logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwsCodeKind {
    /// The API session is over or cannot start: 326 (client id in use), 502 (could not connect), 503 (TWS out of date), 504 (not
    /// connected), 507 (bad message length: end of the socket), 1300 (socket port reset, the connection is dropped).
    SessionEnd,
    /// 1100: TWS lost its connection to the IB servers (internet, the nightly server reset or a competing session). We keep the
    /// session: IB's own client only disconnects on socket errors, and if TWS does close the socket the next read fails anyway.
    IbLinkLost,
    /// 1101 (market data requests lost) and 1102 (kept): the connection to IB is back.
    IbLinkRestored {
        /// 1101: the market data requests were lost and have to be sent again.
        data_lost: bool,
    },
    /// 2110: "Connectivity between TWS and server is broken. It will be restored automatically."
    ServerLinkBroken,
    /// 2103 (market data farm) or 2105 (historical data farm) disconnected.
    FarmDown,
    /// 2104, 2106, 2158: a farm connection is OK (informational).
    FarmUp,
    /// 2107, 2108: a farm connection became inactive but is available on demand (a normal state).
    FarmIdle,
    /// 2100 (the account updates of this client were replaced by another request) and 2101 (another client is subscribed to a
    /// different account): the portfolio values of this app no longer come.
    AccountUpdatesStopped,
    /// 100 (the limit of messages per second exceeded, TWS may disconnect), 10197 (no market data during a competing session).
    Warning,
    /// Not on the page with a meaning this app could act on.
    Unclassified,
}

/// The kind of `code` (see `TwsCodeKind`).
#[must_use]
pub fn classify_tws_code(code: i32) -> TwsCodeKind {
    match code {
        326 | 502 | 503 | 504 | 507 | 1300 => TwsCodeKind::SessionEnd,
        1100 => TwsCodeKind::IbLinkLost,
        1101 => TwsCodeKind::IbLinkRestored { data_lost: true },
        1102 => TwsCodeKind::IbLinkRestored { data_lost: false },
        2110 => TwsCodeKind::ServerLinkBroken,
        2103 | 2105 => TwsCodeKind::FarmDown,
        2104 | 2106 | 2158 => TwsCodeKind::FarmUp,
        2107 | 2108 => TwsCodeKind::FarmIdle,
        2100 | 2101 => TwsCodeKind::AccountUpdatesStopped,
        100 | 10197 => TwsCodeKind::Warning,
        _ => TwsCodeKind::Unclassified,
    }
}

/// TWS's live-price farm notices: 2103 "Market data farm connection is broken:usfarm" and 2104
/// "... is OK:usfarm" (the farm name follows the last ':'; the text itself is localized).
/// Historical (2105/2106) and other notices are not about live prices -> `None`.
#[must_use]
pub fn market_data_farm_status(code: i32, message: &str) -> Option<(String, bool)> {
    let ok = match code {
        2103 => false,
        2104 => true,
        _ => return None,
    };
    let farm = message.rsplit(':').next()?.trim();
    (!farm.is_empty()).then(|| (farm.to_string(), ok))
}

/// TWS writes non-ASCII characters of its (localized) messages as Java `\uXXXX` escapes, e.g.
/// "während" for "während"; this turns them back into the characters.
pub fn decode_unicode_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("\\u") {
        out.push_str(rest.get(..pos).unwrap_or_default());
        let after = rest.get(pos + 2..).unwrap_or_default();
        let decoded = after.get(..4).and_then(|hex| u32::from_str_radix(hex, 16).ok()).and_then(char::from_u32);
        if let Some(c) = decoded {
            out.push(c);
            rest = after.get(4..).unwrap_or_default();
        } else {
            out.push_str("\\u");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// `IBApi.EDecoder.ExecutionDataEvent`'s real field order. At server version >= 136 the leading
/// "version" field disappears from the wire entirely and every `num >= X` gate below effectively
/// becomes `server_version >= X` - below 136 it's still an explicit int read off the wire, exactly
/// like `tws_error_message`'s version field vanishing at >=194. Every field is walked in order
/// (even ones this app doesn't keep) since later fields' positions depend on it.
///
/// # Errors
///
/// The reason as text when the message stops short or a field is invalid.
pub fn parse_execution_data(fields: &[String], server_version: i32) -> Result<ExecutionRow, String> {
    fn take<'a>(fields: &'a [String], i: &mut usize) -> Result<&'a str, String> {
        let v = fields.get(*i).map(String::as_str).ok_or_else(|| "Malformed TWS execution message: missing field".to_string())?;
        *i += 1;
        Ok(v)
    }
    fn take_f64(fields: &[String], i: &mut usize) -> Result<f64, String> {
        take(fields, i)?.parse().map_err(|_| "Malformed TWS execution message: expected a number".to_string())
    }
    fn take_i32(fields: &[String], i: &mut usize) -> Result<i32, String> {
        take(fields, i)?.parse().map_err(|_| "Malformed TWS execution message: expected an integer".to_string())
    }

    let mut i = 1usize; // fields[0] is the message id ("11")
    let num = if server_version < 136 { take_i32(fields, &mut i)? } else { server_version };
    if num >= 7 {
        take(fields, &mut i)?; // reqId - this app has only one standing reqExecutions subscription
    }
    let order_id: i32 = take(fields, &mut i)?.parse().unwrap_or(0);
    let mut con_id = 0;
    if num >= 5 {
        con_id = take(fields, &mut i)?.parse().unwrap_or(0);
    }
    let symbol = take(fields, &mut i)?.to_string();
    let sec_type = take(fields, &mut i)?.to_string();
    let expiry = take(fields, &mut i)?.to_string();
    let strike = take_f64(fields, &mut i)?;
    let right = take(fields, &mut i)?.to_string();
    if num >= 9 {
        take(fields, &mut i)?; // multiplier
    }
    take(fields, &mut i)?; // contract exchange
    take(fields, &mut i)?; // currency
    take(fields, &mut i)?; // local symbol
    if num >= 10 {
        take(fields, &mut i)?; // trading class
    }
    let exec_id = take(fields, &mut i)?.to_string();
    let time = take(fields, &mut i)?.to_string();
    let account = take(fields, &mut i)?.to_string();
    take(fields, &mut i)?; // execution's own exchange
    let side = take(fields, &mut i)?.to_string();
    let quantity = take_f64(fields, &mut i)?;
    let price = take_f64(fields, &mut i)?;
    if num >= 2 {
        take(fields, &mut i)?; // permId
    }
    if num >= 3 {
        take(fields, &mut i)?; // clientId
    }
    if num >= 4 {
        take(fields, &mut i)?; // liquidation
    }
    if num >= 6 {
        take(fields, &mut i)?; // cumQty
        take(fields, &mut i)?; // avgPrice
    }
    let mut order_ref = String::new();
    if num >= 8 {
        order_ref = take(fields, &mut i)?.to_string();
    }
    if num >= 9 {
        take(fields, &mut i)?; // evRule
        take(fields, &mut i)?; // evMultiplier
    }
    if server_version >= 103 {
        take(fields, &mut i)?; // modelCode
    }
    if server_version >= 136 {
        take(fields, &mut i)?; // lastLiquidity
    }
    if server_version >= 178 {
        take(fields, &mut i)?; // pendingPriceRevision
    }
    if server_version >= 198 {
        take(fields, &mut i)?; // submitter
    }
    Ok(ExecutionRow { account, symbol, con_id, time, exec_id, order_id, order_ref, expiry, sec_type, right, strike, side, quantity, price })
}
