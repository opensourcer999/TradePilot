//! The SQLite database of one trading mode (`tradepilot.db` in the mode's folder): the trades
//! this app opened and every execution TWS reported. One local file, no server. Prices are
//! `Decimal` and stored as decimal text (`coding_guideline.md` section 3: no floats for money); the TWS-side numbers of an
//! execution (strike, quantity, price) are `REAL`, as TWS sends them.
//!
//! Files written by earlier versions (`trades.json`, `executions.jsonl`) next to the database
//! are imported once, then renamed to `*.migrated`.

use crate::storage::{StoredExecution, StoredLeg, StoredTrade};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, Row};
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

const DB_FILE_NAME: &str = "tradepilot.db";

/// The columns of the `trades` table, in the order `trade_values` and `trade_from_row` use.
const TRADE_COLUMNS: [&str; 40] = [
    "day",
    "opened",
    "account",
    "symbol",
    "trade_type",
    "strikes",
    "order_id",
    "qty",
    "multiplier",
    "price_open",
    "delta",
    "short_put_con",
    "short_put_strike",
    "long_put_con",
    "long_put_strike",
    "short_call_con",
    "short_call_strike",
    "long_call_con",
    "long_call_strike",
    "put_ratio",
    "call_ratio",
    "expiry",
    "status",
    "price_close",
    "stop_order_id",
    "profit_order_id",
    "close_order_id",
    "stop_price",
    "profit_price",
    "entry_unknown",
    "stop_limit_order_id",
    "exit_levels",
    "adjustments",
    "oca_group",
    "strategy",
    "timed_exit",
    "profit_cancel",
    "origin",
    "custom_legs",
    "id",
];

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS trades (
    id INTEGER PRIMARY KEY,
    day TEXT NOT NULL,
    opened TEXT NOT NULL,
    account TEXT NOT NULL,
    symbol TEXT NOT NULL,
    trade_type TEXT NOT NULL,
    strikes TEXT NOT NULL,
    order_id INTEGER NOT NULL,
    qty INTEGER NOT NULL,
    multiplier INTEGER NOT NULL,
    price_open TEXT NOT NULL,
    delta REAL NOT NULL,
    short_put_con INTEGER, short_put_strike TEXT,
    long_put_con INTEGER, long_put_strike TEXT,
    short_call_con INTEGER, short_call_strike TEXT,
    long_call_con INTEGER, long_call_strike TEXT,
    put_ratio INTEGER NOT NULL,
    call_ratio INTEGER NOT NULL,
    expiry TEXT NOT NULL,
    status TEXT NOT NULL,
    price_close TEXT,
    stop_order_id INTEGER, profit_order_id INTEGER, close_order_id INTEGER,
    stop_price TEXT, profit_price TEXT,
    entry_unknown INTEGER NOT NULL DEFAULT 0,
    stop_limit_order_id INTEGER,
    exit_levels TEXT,
    adjustments TEXT,
    oca_group TEXT,
    strategy TEXT NOT NULL DEFAULT '',
    timed_exit TEXT,
    profit_cancel TEXT,
    origin TEXT,
    custom_legs TEXT
);
CREATE INDEX IF NOT EXISTS trades_day ON trades (day);
CREATE INDEX IF NOT EXISTS trades_status ON trades (status);
CREATE TABLE IF NOT EXISTS executions (
    exec_id TEXT PRIMARY KEY,
    time TEXT NOT NULL,
    account TEXT NOT NULL,
    symbol TEXT NOT NULL,
    con_id INTEGER NOT NULL,
    order_id INTEGER NOT NULL,
    order_ref TEXT NOT NULL,
    sec_type TEXT NOT NULL,
    expiry TEXT NOT NULL,
    opt_right TEXT NOT NULL,
    strike REAL NOT NULL,
    side TEXT NOT NULL,
    quantity REAL NOT NULL,
    price REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS executions_order_ref ON executions (order_ref);
CREATE TABLE IF NOT EXISTS commissions (
    exec_id TEXT PRIMARY KEY,
    amount TEXT NOT NULL
);
";

/// The database file of a mode folder.
pub fn db_path(mode_dir: &Path) -> std::path::PathBuf {
    mode_dir.join(DB_FILE_NAME)
}

fn io_error(error: rusqlite::Error) -> io::Error {
    io::Error::other(error)
}

/// Opens (and if needed creates and migrates) the database at `path`.
pub fn open(path: &Path) -> io::Result<Connection> {
    open_counting(path).map(|(connection, _, _)| connection)
}

/// `open`, also telling how many trades and executions were imported from the old files.
fn open_counting(path: &Path) -> io::Result<(Connection, usize, usize)> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path).map_err(io_error)?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(io_error)?;
    connection.execute_batch(SCHEMA).map_err(io_error)?;
    add_missing_columns(&connection)?;
    let (mut trades, mut executions) = (0, 0);
    if path.file_name().is_some_and(|name| name == DB_FILE_NAME) {
        if let Some(dir) = path.parent() {
            (trades, executions) = migrate_files(&connection, dir)?;
        }
    }
    Ok((connection, trades, executions))
}

/// Databases created by earlier versions miss columns added later: they are added here.
fn add_missing_columns(connection: &Connection) -> io::Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(trades)").map_err(io_error)?;
    let names: Vec<String> = statement.query_map([], |row| row.get::<_, String>(1)).map_err(io_error)?.collect::<rusqlite::Result<_>>().map_err(io_error)?;
    for (column, kind) in [
        ("stop_limit_order_id", "INTEGER"),
        ("exit_levels", "TEXT"),
        ("adjustments", "TEXT"),
        ("oca_group", "TEXT"),
        ("strategy", "TEXT NOT NULL DEFAULT ''"),
        ("timed_exit", "TEXT"),
        ("profit_cancel", "TEXT"),
        ("origin", "TEXT"),
        ("custom_legs", "TEXT"),
    ] {
        if !names.iter().any(|name| name == column) {
            connection.execute(&format!("ALTER TABLE trades ADD COLUMN {column} {kind}"), []).map_err(io_error)?;
        }
    }
    Ok(())
}

/// What opening the database at start-up found, for the log.
#[derive(Debug, PartialEq, Eq)]
pub struct InitReport {
    /// The database file did not exist and has been created with an empty schema.
    pub created: bool,
    pub trades: usize,
    pub executions: usize,
    /// Imported from `trades.json` / `executions.jsonl` of earlier versions just now.
    pub migrated_trades: usize,
    pub migrated_executions: usize,
}

impl InitReport {
    /// The log lines describing the initialisation of the database at `path`.
    pub fn log_lines(&self, path: &Path) -> Vec<String> {
        let state = if self.created {
            "created with an empty schema (tables trades, executions)".to_string()
        } else {
            format!("opened ({} trade(s), {} execution(s) stored)", self.trades, self.executions)
        };
        let mut lines = vec![format!("SQLite {} database {}: {state}", rusqlite::version(), path.display())];
        if self.migrated_trades > 0 || self.migrated_executions > 0 {
            lines.push(format!(
                "SQLite: imported {} trade(s) and {} execution(s) from the files of an earlier version (renamed to *.migrated)",
                self.migrated_trades, self.migrated_executions
            ));
        }
        lines
    }
}

/// Opens the database at `path`, creating and migrating it if needed, and reports what it found.
pub fn init(path: &Path) -> io::Result<InitReport> {
    let created = !path.exists();
    let (connection, migrated_trades, migrated_executions) = open_counting(path)?;
    let count = |table: &str| -> io::Result<usize> {
        let n: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).map_err(io_error)?;
        Ok(usize::try_from(n).unwrap_or(0))
    };
    Ok(InitReport { created, trades: count("trades")?, executions: count("executions")?, migrated_trades, migrated_executions })
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn opt_text(value: Option<&String>) -> Value {
    value.map_or(Value::Null, |v| Value::Text(v.clone()))
}

/// A price as its decimal text (`Decimal`'s own text: nothing is rounded).
fn decimal(value: rust_decimal::Decimal) -> Value {
    Value::Text(value.to_string())
}

fn opt_decimal(value: Option<rust_decimal::Decimal>) -> Value {
    value.map_or(Value::Null, decimal)
}

/// The price in column `index`, stored as decimal text. Text that is no decimal is an error of
/// the row, not a price of 0.
fn decimal_at(row: &Row<'_>, index: usize) -> rusqlite::Result<rust_decimal::Decimal> {
    let text: String = row.get(index)?;
    text.trim().parse().map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error)))
}

fn opt_decimal_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<rust_decimal::Decimal>> {
    let text: Option<String> = row.get(index)?;
    text.map(|text| text.trim().parse().map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))))
        .transpose()
}

fn opt_int(value: Option<i32>) -> Value {
    value.map_or(Value::Null, |v| Value::Integer(i64::from(v)))
}

/// The values of one trade for `TRADE_COLUMNS` (the trailing "id" is not bound).
fn trade_values(t: &StoredTrade) -> Vec<Value> {
    let leg_con = |leg: &Option<StoredLeg>| opt_int(leg.as_ref().map(|l| l.con_id));
    let leg_strike = |leg: &Option<StoredLeg>| opt_text(leg.as_ref().map(|l| &l.strike));
    vec![
        text(&t.day),
        text(&t.opened),
        text(&t.account),
        text(&t.symbol),
        text(&t.trade_type),
        text(&t.strikes),
        Value::Integer(i64::from(t.order_id)),
        Value::Integer(i64::from(t.qty)),
        Value::Integer(i64::from(t.multiplier)),
        decimal(t.price_open),
        Value::Real(t.delta),
        leg_con(&t.short_put),
        leg_strike(&t.short_put),
        leg_con(&t.long_put),
        leg_strike(&t.long_put),
        leg_con(&t.short_call),
        leg_strike(&t.short_call),
        leg_con(&t.long_call),
        leg_strike(&t.long_call),
        Value::Integer(i64::from(t.put_ratio)),
        Value::Integer(i64::from(t.call_ratio)),
        text(&t.expiry),
        text(&t.status),
        opt_decimal(t.price_close),
        opt_int(t.stop_order_id),
        opt_int(t.profit_order_id),
        opt_int(t.close_order_id),
        opt_decimal(t.stop_price),
        opt_decimal(t.profit_price),
        Value::Integer(i64::from(t.entry_unknown)),
        opt_int(t.stop_limit_order_id),
        opt_text(t.exit_levels.as_ref().and_then(|levels| serde_json::to_string(levels).ok()).as_ref()),
        opt_text(t.adjustments.as_ref().and_then(|adjustments| serde_json::to_string(adjustments).ok()).as_ref()),
        opt_text(t.oca_group.as_ref()),
        text(&t.strategy),
        opt_text(t.timed_exit.as_ref().and_then(|exit| serde_json::to_string(exit).ok()).as_ref()),
        opt_text(t.profit_cancel.as_ref()),
        opt_text(t.origin.as_ref().and_then(|origin| serde_json::to_string(origin).ok()).as_ref()),
        opt_text(t.custom_legs.as_ref().and_then(|legs| serde_json::to_string(legs).ok()).as_ref()),
    ]
}

fn trade_from_row(row: &Row<'_>) -> rusqlite::Result<StoredTrade> {
    let leg = |con: usize, strike: usize| -> rusqlite::Result<Option<StoredLeg>> {
        let con_id: Option<i32> = row.get(con)?;
        let strike: Option<String> = row.get(strike)?;
        Ok(con_id.zip(strike).map(|(con_id, strike)| StoredLeg { con_id, strike }))
    };
    Ok(StoredTrade {
        day: row.get(0)?,
        opened: row.get(1)?,
        account: row.get(2)?,
        symbol: row.get(3)?,
        trade_type: row.get(4)?,
        strikes: row.get(5)?,
        order_id: row.get(6)?,
        qty: row.get(7)?,
        multiplier: row.get(8)?,
        price_open: decimal_at(row, 9)?,
        delta: row.get(10)?,
        short_put: leg(11, 12)?,
        long_put: leg(13, 14)?,
        short_call: leg(15, 16)?,
        long_call: leg(17, 18)?,
        put_ratio: row.get(19)?,
        call_ratio: row.get(20)?,
        expiry: row.get(21)?,
        status: row.get(22)?,
        price_close: opt_decimal_at(row, 23)?,
        stop_order_id: row.get(24)?,
        profit_order_id: row.get(25)?,
        close_order_id: row.get(26)?,
        stop_price: opt_decimal_at(row, 27)?,
        profit_price: opt_decimal_at(row, 28)?,
        entry_unknown: row.get::<_, i64>(29)? != 0,
        stop_limit_order_id: row.get(30)?,
        exit_levels: row.get::<_, Option<String>>(31)?.and_then(|text| serde_json::from_str(&text).ok()),
        adjustments: row.get::<_, Option<String>>(32)?.and_then(|text| serde_json::from_str(&crate::legacy_names::upgrade_adjustments(&text)).ok()),
        oca_group: row.get(33)?,
        strategy: row.get(34)?,
        timed_exit: row.get::<_, Option<String>>(35)?.and_then(|text| serde_json::from_str(&text).ok()),
        profit_cancel: row.get(36)?,
        origin: row.get::<_, Option<String>>(37)?.and_then(|text| serde_json::from_str(&text).ok()),
        custom_legs: row.get::<_, Option<String>>(38)?.and_then(|text| serde_json::from_str(&text).ok()),
    })
}

/// The columns without the trailing "id".
fn data_columns() -> &'static [&'static str] {
    TRADE_COLUMNS.split_last().map_or(&[], |(_, rest)| rest)
}

fn insert_trade(connection: &Connection, trade: &StoredTrade) -> rusqlite::Result<()> {
    let columns = data_columns();
    let marks: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
    let sql = format!("INSERT INTO trades ({}) VALUES ({})", columns.join(", "), marks.join(", "));
    connection.execute(&sql, params_from_iter(trade_values(trade)))?;
    Ok(())
}

/// Every stored trade, in the order they were stored.
pub fn load_trades(path: &Path) -> io::Result<Vec<StoredTrade>> {
    let connection = open(path)?;
    let sql = format!("SELECT {} FROM trades ORDER BY id", data_columns().join(", "));
    let mut statement = connection.prepare(&sql).map_err(io_error)?;
    let rows = statement.query_map([], trade_from_row).map_err(io_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(io_error)
}

/// Stores one trade.
pub fn append_trade(path: &Path, trade: &StoredTrade) -> io::Result<()> {
    insert_trade(&open(path)?, trade).map_err(io_error)
}

/// Replaces the stored copy of `trade` (same day, order, trade type and strikes).
pub fn update_trade(path: &Path, trade: &StoredTrade) -> io::Result<()> {
    let connection = open(path)?;
    let columns = data_columns();
    let sets: Vec<String> = columns.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 1)).collect();
    let position = |name: &str| columns.iter().position(|c| *c == name).map_or(1, |i| i + 1);
    let sql = format!(
        "UPDATE trades SET {} WHERE day = ?{} AND order_id = ?{} AND trade_type = ?{} AND strikes = ?{}",
        sets.join(", "),
        position("day"),
        position("order_id"),
        position("trade_type"),
        position("strikes")
    );
    connection.execute(&sql, params_from_iter(trade_values(trade))).map_err(io_error)?;
    Ok(())
}

fn insert_execution(connection: &Connection, e: &StoredExecution) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT OR IGNORE INTO executions (exec_id, time, account, symbol, con_id, order_id, order_ref, sec_type, expiry, opt_right, strike, side, quantity, price)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![e.exec_id, e.time, e.account, e.symbol, e.con_id, e.order_id, e.order_ref, e.sec_type, e.expiry, e.right, e.strike, e.side, e.quantity, e.price],
    )
}

/// Every stored execution, in the order they were stored.
pub fn load_executions(path: &Path) -> io::Result<Vec<StoredExecution>> {
    let connection = open(path)?;
    let mut statement = connection
        .prepare("SELECT exec_id, time, account, symbol, con_id, order_id, order_ref, sec_type, expiry, opt_right, strike, side, quantity, price FROM executions ORDER BY rowid")
        .map_err(io_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok(StoredExecution {
                exec_id: r.get(0)?,
                time: r.get(1)?,
                account: r.get(2)?,
                symbol: r.get(3)?,
                con_id: r.get(4)?,
                order_id: r.get(5)?,
                order_ref: r.get(6)?,
                sec_type: r.get(7)?,
                expiry: r.get(8)?,
                right: r.get(9)?,
                strike: r.get(10)?,
                side: r.get(11)?,
                quantity: r.get(12)?,
                price: r.get(13)?,
            })
        })
        .map_err(io_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(io_error)
}

/// Stores one execution; one with a known id is ignored.
pub fn append_execution(path: &Path, execution: &StoredExecution) -> io::Result<()> {
    insert_execution(&open(path)?, execution).map(|_| ()).map_err(io_error)
}

/// Stores the commission (dollars, as text) TWS charged for the execution `exec_id`; a later report replaces it.
pub fn save_commission(path: &Path, exec_id: &str, amount: rust_decimal::Decimal) -> io::Result<()> {
    open(path)?
        .execute("INSERT OR REPLACE INTO commissions (exec_id, amount) VALUES (?1, ?2)", params![exec_id, amount.to_string()])
        .map(|_| ())
        .map_err(io_error)
}

/// Every stored commission by execution id; an unreadable amount is skipped.
pub fn load_commissions(path: &Path) -> io::Result<std::collections::HashMap<String, rust_decimal::Decimal>> {
    let connection = open(path)?;
    let mut statement = connection.prepare("SELECT exec_id, amount FROM commissions").map_err(io_error)?;
    let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(io_error)?;
    let all = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(io_error)?;
    Ok(all.into_iter().filter_map(|(id, amount)| amount.parse().ok().map(|amount| (id, amount))).collect())
}

/// Imports `trades.json` and `executions.jsonl` of earlier versions once and renames them.
fn migrate_files(connection: &Connection, dir: &Path) -> io::Result<(usize, usize)> {
    let trades_file = dir.join("trades.json");
    let executions_file = dir.join("executions.jsonl");
    let old_trades: Vec<StoredTrade> = fs::read_to_string(&trades_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let old_executions: Vec<StoredExecution> =
        fs::read_to_string(&executions_file).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default();
    if !trades_file.exists() && !executions_file.exists() {
        return Ok((0, 0));
    }
    connection.execute_batch("BEGIN").map_err(io_error)?;
    let result = (|| -> rusqlite::Result<()> {
        for trade in &old_trades {
            insert_trade(connection, trade)?;
        }
        for execution in &old_executions {
            insert_execution(connection, execution)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => connection.execute_batch("COMMIT").map_err(io_error)?,
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(io_error(error));
        }
    }
    for file in [trades_file, executions_file] {
        if file.exists() {
            let mut target = file.clone().into_os_string();
            target.push(".migrated");
            fs::rename(&file, target)?;
        }
    }
    Ok((old_trades.len(), old_executions.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn temp_dir(name: &str) -> TempDir {
        TempDir::new(&format!("db-{name}"))
    }

    fn trade(order_id: i32) -> StoredTrade {
        StoredTrade {
            day: "2026-11-13".into(),
            opened: "13-11-2026 15:30:00".into(),
            account: "DU123".into(),
            symbol: "SPX".into(),
            trade_type: "PutSpread".into(),
            strikes: "6650/6640 P".into(),
            order_id,
            qty: 2,
            multiplier: 100,
            price_open: rust_decimal::Decimal::from_str_exact("1.15").unwrap(),
            delta: 4.5,
            short_put: Some(StoredLeg { con_id: 11, strike: "6650".into() }),
            long_put: Some(StoredLeg { con_id: 12, strike: "6640".into() }),
            short_call: None,
            long_call: None,
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261113".into(),
            status: "Open".into(),
            price_close: None,
            stop_order_id: Some(21),
            profit_order_id: Some(22),
            close_order_id: None,
            stop_price: Some(rust_decimal::Decimal::from_str_exact("3.45").unwrap()),
            profit_price: Some(rust_decimal::Decimal::from_str_exact("0.55").unwrap()),
            entry_unknown: false,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
            stop_limit_order_id: Some(23),
            exit_levels: Some(crate::storage::ExitLevels {
                qty: [1, 1, 0, 0],
                level: 1,
                order_ids: [Some(22), Some(24), None, None],
                closed: vec![(1, "-0.55".into())],
                prices: [Some("0.55".into()), Some("0.3".into()), None, None],
                legs: vec![(11, 1, false), (12, 1, true)],
                outside_rth: true,
            }),
            adjustments: Some(crate::storage::StopAdjustments {
                items: vec![crate::storage::StopAdjustment {
                    kind: "Profit percent".into(),
                    value: Some(50.0),
                    order_type: "Same".into(),
                    change_type: "Stop factor".into(),
                    change: Some(0.0),
                    change_offset: Some(0.1),
                    hour: 15,
                    minute: 30,
                }],
                level: 1,
                order_type: "Limit stop".into(),
                stop: Some("3.45".into()),
                trailing_offset: None,
                limit_offset: Some(0.2),
                market_offset: None,
                legs: vec![(11, 1, false)],
                trigger_method: 4,
                outside_rth: false,
                replace_pending: true,
                rel: None,
                basis: None,
            }),
            oca_group: Some("TradePilot-21".into()),
        }
    }

    fn execution(id: &str) -> StoredExecution {
        StoredExecution {
            exec_id: id.into(),
            time: "20261113 21:30:05 Europe/Berlin".into(),
            account: "DU123".into(),
            symbol: "SPX".into(),
            con_id: 11,
            order_id: 7,
            order_ref: "TraPi-7".into(),
            sec_type: "OPT".into(),
            expiry: "20261113".into(),
            right: "P".into(),
            strike: 7535.0,
            side: "SLD".into(),
            quantity: 1.0,
            price: 1.4,
        }
    }

    #[test]
    fn a_trade_is_stored_and_read_back_exactly_and_updated_in_place() {
        let dir = temp_dir("trades");
        let path = db_path(&dir);
        assert_eq!(load_trades(&path).unwrap(), [] as [crate::storage::StoredTrade; 0]);
        append_trade(&path, &trade(7)).unwrap();
        append_trade(&path, &trade(8)).unwrap();
        let back = load_trades(&path).unwrap();
        assert_eq!(back, [trade(7), trade(8)]);

        let mut closed = trade(7);
        closed.status = "Stopped".into();
        closed.price_close = Some(rust_decimal::Decimal::from_str_exact("-3.45").unwrap());
        update_trade(&path, &closed).unwrap();
        let back = load_trades(&path).unwrap();
        assert_eq!(back, [closed, trade(8)], "only the matching trade changed, the order is kept");
    }

    #[test]
    fn a_database_of_an_earlier_version_gets_the_new_columns_and_keeps_its_trades() {
        let dir = temp_dir("old-schema");
        fs::create_dir_all(&dir).unwrap();
        let path = db_path(&dir);
        // The table as it was before the stop-limit order, the profit-target levels, the stop
        // adjustments, the stored OCA group, the strategy, the timed exit and the profit-target cancel time: without their seven columns.
        let old_schema = SCHEMA.replace(
            "entry_unknown INTEGER NOT NULL DEFAULT 0,\n    stop_limit_order_id INTEGER,\n    exit_levels TEXT,\n    adjustments TEXT,\n    oca_group TEXT,\n    strategy TEXT NOT NULL DEFAULT '',\n    timed_exit TEXT,\n    profit_cancel TEXT,\n    origin TEXT,\n    custom_legs TEXT\n);",
            "entry_unknown INTEGER NOT NULL DEFAULT 0\n);",
        );
        assert_ne!(old_schema, SCHEMA, "the old schema really lacks the columns");
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(&old_schema).unwrap();
            connection
                .execute(
                    "INSERT INTO trades (day, opened, account, symbol, trade_type, strikes, order_id, qty, multiplier, price_open, delta, put_ratio, call_ratio, expiry, status)
                     VALUES ('2026-11-13', '13-11-2026 10:00:00', 'DU1', 'SPX', 'PutSpread', '6650/6640 P', 7, 2, 100, '1.15', 4.5, 1, 1, '20261113', 'Open')",
                    [],
                )
                .unwrap();
        }
        // Opening adds the columns; the old trade is read with nothing in them.
        let loaded = load_trades(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        let old = loaded.first().unwrap().clone();
        assert_eq!((old.order_id, old.stop_limit_order_id, old.exit_levels.clone(), old.adjustments.clone()), (7, None, None, None));
        assert_eq!(old.oca_group, None);
        // Opening again does not add them twice, and the new data can be stored and read back.
        let mut updated = old.clone();
        updated.stop_limit_order_id = Some(9);
        updated.exit_levels = trade(7).exit_levels;
        updated.adjustments = trade(7).adjustments;
        updated.oca_group = Some("TradePilot-9".into());
        update_trade(&path, &updated).unwrap();
        assert_eq!(load_trades(&path).unwrap(), [updated]);
    }

    #[test]
    fn commissions_are_stored_by_execution_and_a_later_report_replaces_the_amount() {
        let dir = temp_dir("commissions");
        let path = db_path(&dir);
        assert!(load_commissions(&path).unwrap().is_empty());
        save_commission(&path, "e1", rust_decimal::Decimal::new(65, 2)).unwrap();
        save_commission(&path, "e2", rust_decimal::Decimal::new(70, 2)).unwrap();
        save_commission(&path, "e1", rust_decimal::Decimal::new(66, 2)).unwrap();
        let back = load_commissions(&path).unwrap();
        assert_eq!((back.len(), back["e1"], back["e2"]), (2, rust_decimal::Decimal::new(66, 2), rust_decimal::Decimal::new(70, 2)));
    }

    #[test]
    fn executions_are_stored_once_by_their_id() {
        let dir = temp_dir("exec");
        let path = db_path(&dir);
        append_execution(&path, &execution("a")).unwrap();
        append_execution(&path, &execution("a")).unwrap();
        append_execution(&path, &execution("b")).unwrap();
        let back = load_executions(&path).unwrap();
        assert_eq!(back.iter().map(|e| e.exec_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(back.first(), Some(&execution("a")));
    }

    #[test]
    fn files_of_earlier_versions_are_imported_once_and_renamed() {
        let dir = temp_dir("migrate");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("trades.json"), serde_json::to_string(&[trade(7)]).unwrap()).unwrap();
        let lines: Vec<String> = [execution("a"), execution("b")].iter().map(|e| serde_json::to_string(e).unwrap()).collect();
        fs::write(dir.join("executions.jsonl"), lines.join("\n")).unwrap();

        let path = db_path(&dir);
        assert_eq!(load_trades(&path).unwrap(), [trade(7)]);
        assert_eq!(load_executions(&path).unwrap().len(), 2);
        assert!(!dir.join("trades.json").exists() && dir.join("trades.json.migrated").exists());
        assert!(!dir.join("executions.jsonl").exists() && dir.join("executions.jsonl.migrated").exists());
        // Opening again imports nothing a second time.
        assert_eq!(load_trades(&path).unwrap().len(), 1);
        assert_eq!(load_executions(&path).unwrap().len(), 2);
    }

    #[test]
    fn the_initialisation_reports_a_new_database_then_its_contents_and_imports() {
        let dir = temp_dir("init");
        let path = db_path(&dir);
        let first = init(&path).unwrap();
        assert_eq!(first, InitReport { created: true, trades: 0, executions: 0, migrated_trades: 0, migrated_executions: 0 });
        let lines = first.log_lines(&path);
        assert!(lines.len() == 1 && lines[0].contains("created with an empty schema") && lines[0].contains("SQLite 3."), "{lines:?}");

        append_trade(&path, &trade(7)).unwrap();
        append_execution(&path, &execution("a")).unwrap();
        let again = init(&path).unwrap();
        assert_eq!((again.created, again.trades, again.executions), (false, 1, 1));
        assert!(again.log_lines(&path)[0].contains("opened (1 trade(s), 1 execution(s) stored)"));

        // Files of an earlier version are imported by the first opening, and the log says so.
        let old_dir = temp_dir("init-old");
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(old_dir.join("trades.json"), serde_json::to_string(&[trade(7), trade(8)]).unwrap()).unwrap();
        let old_path = db_path(&old_dir);
        let migrated = init(&old_path).unwrap();
        assert_eq!((migrated.migrated_trades, migrated.migrated_executions, migrated.trades), (2, 0, 2));
        let lines = migrated.log_lines(&old_path);
        assert!(lines.len() == 2 && lines[1].contains("imported 2 trade(s) and 0 execution(s)"), "{lines:?}");
    }

    #[test]
    fn a_trade_from_the_json_of_an_old_version_without_the_newer_fields_still_loads() {
        let dir = temp_dir("old-json");
        fs::create_dir_all(&dir).unwrap();
        let old = r#"[{"day":"2026-11-12","opened":"12-11-2026 10:00:00","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"6650/6640 P","order_id":3,"qty":1,"multiplier":100,"price_open":"1.2","delta":4.0}]"#;
        fs::write(dir.join("trades.json"), old).unwrap();
        let back = load_trades(&db_path(&dir)).unwrap();
        let [t] = back.as_slice() else { panic!("one trade expected") };
        assert_eq!((t.status.as_str(), t.put_ratio, t.entry_unknown, t.short_put.is_none()), ("Open", 1, false, true));
    }
}
