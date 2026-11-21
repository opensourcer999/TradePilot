//! The trade types and what each of them is made of - the one place that knows it
//! (`coding_guideline.md` section 5). Templates and stored trades keep the type as its text
//! (`TradeTemplate::TradeType`, `StoredTrade::trade_type`, as the template files write it);
//! the logic asks a `trade_type` instead of comparing texts. Every question is a
//! `match` without a wildcard arm, so a new type does not compile until it is placed everywhere.

/// One trade type, by its name in the template files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TradeType {
    /// A sold put and a bought put below it (credit).
    PutSpread,
    /// A sold call and a bought call above it (credit).
    CallSpread,
    /// One sold put.
    ShortPut,
    /// One sold call.
    ShortCall,
    /// One bought put.
    LongPut,
    /// One bought call.
    LongCall,
    /// A put spread and a call spread as one trade.
    IronCondor,
    /// A put spread and a call spread entered as one order, kept as two trades.
    IronCondorFull,
    /// An iron condor whose two shorts share the strike nearest the price.
    IronFly,
    /// A sold put and a sold call.
    ShortStrangle,
    /// A bought put and a bought call.
    LongStrangle,
    /// A bought put and a sold put below it (debit).
    PutDebitSpread,
    /// A bought call and a sold call above it (debit).
    CallDebitSpread,
    /// A put debit spread and a call debit spread as one order.
    LongIronCondorFull,
    /// A sold near put and a bought far put (debit).
    PutCalendar,
    /// A sold near call and a bought far call (debit).
    CallCalendar,
    /// A put calendar and a call calendar as one trade.
    DoubleCalendar,
    /// A sold put and a sold call at the same strike (not entered by this app).
    Straddle,
    /// Up to four freely defined legs (not entered by this app yet).
    Custom,
}

impl TradeType {
    /// Every trade type.
    pub const ALL: [Self; 19] = [
        Self::PutSpread,
        Self::CallSpread,
        Self::ShortPut,
        Self::ShortCall,
        Self::LongPut,
        Self::LongCall,
        Self::IronCondor,
        Self::IronCondorFull,
        Self::IronFly,
        Self::ShortStrangle,
        Self::LongStrangle,
        Self::PutDebitSpread,
        Self::CallDebitSpread,
        Self::LongIronCondorFull,
        Self::PutCalendar,
        Self::CallCalendar,
        Self::DoubleCalendar,
        Self::Straddle,
        Self::Custom,
    ];

    /// The name the template files use ("IronCondor-Full" ...).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PutSpread => "PutSpread",
            Self::CallSpread => "CallSpread",
            Self::ShortPut => "ShortPut",
            Self::ShortCall => "ShortCall",
            Self::LongPut => "LongPut",
            Self::LongCall => "LongCall",
            Self::IronCondor => "IronCondor",
            Self::IronCondorFull => "IronCondor-Full",
            Self::IronFly => "IronFly",
            Self::ShortStrangle => "ShortStrangle",
            Self::LongStrangle => "LongStrangle",
            Self::PutDebitSpread => "PutDebitSpread",
            Self::CallDebitSpread => "CallDebitSpread",
            Self::LongIronCondorFull => "LongIronCondor-Full",
            Self::PutCalendar => "PutCalendar",
            Self::CallCalendar => "CallCalendar",
            Self::DoubleCalendar => "DoubleCalendar",
            Self::Straddle => "Straddle",
            Self::Custom => "Custom",
        }
    }

    /// The type of a name, exactly as the template files write it; `None` for any other text.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    /// Bought for a debit (`TradeTemplate.IsDebit`): everything but the sold types.
    #[must_use]
    pub fn is_debit(self) -> bool {
        match self {
            Self::PutSpread
            | Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::Straddle => false,
            Self::LongPut
            | Self::LongCall
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::CallDebitSpread
            | Self::LongIronCondorFull
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Custom => true,
        }
    }

    /// Near and far legs of different expirations (entered by `select_calendar`).
    #[must_use]
    pub fn is_calendar(self) -> bool {
        match self {
            Self::PutCalendar | Self::CallCalendar | Self::DoubleCalendar => true,
            Self::PutSpread
            | Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::LongPut
            | Self::LongCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::CallDebitSpread
            | Self::LongIronCondorFull
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// Entered through the tranche entry (`select_trade`): every type of one
    /// expiration this app enters. A straddle is not entered at all, a calendar has its own entry.
    #[must_use]
    pub fn uses_tranche_entry(self) -> bool {
        match self {
            Self::PutSpread
            | Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::LongPut
            | Self::LongCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::CallDebitSpread
            | Self::LongIronCondorFull => true,
            Self::PutCalendar | Self::CallCalendar | Self::DoubleCalendar | Self::Straddle | Self::Custom => false,
        }
    }

    /// Only sold options: a short put, a short call or both.
    #[must_use]
    pub fn sells_only(self) -> bool {
        match self {
            Self::ShortPut | Self::ShortCall | Self::ShortStrangle => true,
            Self::PutSpread
            | Self::CallSpread
            | Self::LongPut
            | Self::LongCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::CallDebitSpread
            | Self::LongIronCondorFull
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// Only bought options: a long put, a long call or both.
    #[must_use]
    pub fn buys_only(self) -> bool {
        match self {
            Self::LongPut | Self::LongCall | Self::LongStrangle => true,
            Self::PutSpread
            | Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::PutDebitSpread
            | Self::CallDebitSpread
            | Self::LongIronCondorFull
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// A debit spread: the bought option is the one nearer the money, the sold one is further out.
    #[must_use]
    pub fn is_debit_spread(self) -> bool {
        match self {
            Self::PutDebitSpread | Self::CallDebitSpread | Self::LongIronCondorFull => true,
            Self::PutSpread
            | Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::LongPut
            | Self::LongCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// The bought options are what the trade is about, so its exit always sells them: the long
    /// options and the debit spreads.
    #[must_use]
    pub fn closes_longs_always(self) -> bool {
        self.buys_only() || self.is_debit_spread()
    }

    /// A single bought option.
    #[must_use]
    pub fn is_single_long(self) -> bool {
        matches!(self, Self::LongPut | Self::LongCall)
    }

    /// Built from put verticals (a sold and a bought put of one expiration) in the tranche entry.
    #[must_use]
    pub fn builds_put_verticals(self) -> bool {
        match self {
            Self::PutSpread | Self::IronCondor | Self::IronCondorFull | Self::IronFly | Self::PutDebitSpread | Self::LongIronCondorFull => true,
            Self::CallSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::LongPut
            | Self::LongCall
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::CallDebitSpread
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// Built from call verticals in the tranche entry.
    #[must_use]
    pub fn builds_call_verticals(self) -> bool {
        match self {
            Self::CallSpread | Self::IronCondor | Self::IronCondorFull | Self::IronFly | Self::CallDebitSpread | Self::LongIronCondorFull => true,
            Self::PutSpread
            | Self::ShortPut
            | Self::ShortCall
            | Self::LongPut
            | Self::LongCall
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::PutCalendar
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle
            | Self::Custom => false,
        }
    }

    /// Has a put leg (or legs).
    #[must_use]
    pub fn has_put_side(self) -> bool {
        match self {
            Self::PutSpread
            | Self::ShortPut
            | Self::LongPut
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::PutDebitSpread
            | Self::LongIronCondorFull
            | Self::PutCalendar
            | Self::DoubleCalendar
            | Self::Straddle => true,
            Self::CallSpread | Self::ShortCall | Self::LongCall | Self::CallDebitSpread | Self::CallCalendar | Self::Custom => false,
        }
    }

    /// Has a call leg (or legs).
    #[must_use]
    pub fn has_call_side(self) -> bool {
        match self {
            Self::CallSpread
            | Self::ShortCall
            | Self::LongCall
            | Self::IronCondor
            | Self::IronCondorFull
            | Self::IronFly
            | Self::ShortStrangle
            | Self::LongStrangle
            | Self::CallDebitSpread
            | Self::LongIronCondorFull
            | Self::CallCalendar
            | Self::DoubleCalendar
            | Self::Straddle => true,
            Self::PutSpread | Self::ShortPut | Self::LongPut | Self::PutDebitSpread | Self::PutCalendar | Self::Custom => false,
        }
    }
}

impl std::fmt::Display for TradeType {
    /// The name the template files use (`as_str`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `TradeType::is_debit` of a type's name; a name that is not a known type counts as a debit, as
/// the list of sold types it is checked against does not name it.
pub fn is_debit(name: &str) -> bool {
    TradeType::parse(name).is_none_or(TradeType::is_debit)
}

/// Whether the type named `name` has the property `property`; `false` for an unknown name.
pub fn is(name: &str, property: fn(TradeType) -> bool) -> bool {
    TradeType::parse(name).is_some_and(property)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_found_by_its_name_and_nothing_else_is() {
        for kind in TradeType::ALL {
            assert_eq!(TradeType::parse(kind.as_str()), Some(kind));
        }
        for unknown in ["", "ironcondor-full", "IronCondorFull", "Iron Condor", "PutSpread "] {
            assert_eq!(TradeType::parse(unknown), None, "{unknown:?}");
        }
        let mut names: Vec<&str> = TradeType::ALL.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TradeType::ALL.len(), "every name is different");
    }

    /// The names of the types with `property`.
    fn named(property: fn(TradeType) -> bool) -> Vec<&'static str> {
        TradeType::ALL.into_iter().filter(|k| property(*k)).map(TradeType::as_str).collect()
    }

    #[test]
    fn the_properties_name_exactly_the_types_of_the_lists_they_replace() {
        // Credit: what `TradeTemplate.IsDebit` lists as not debit.
        let credit: Vec<&str> = TradeType::ALL.into_iter().filter(|k| !k.is_debit()).map(TradeType::as_str).collect();
        assert_eq!(credit, ["PutSpread", "CallSpread", "ShortPut", "ShortCall", "IronCondor", "IronCondor-Full", "IronFly", "ShortStrangle", "Straddle"]);
        assert_eq!(named(TradeType::is_calendar), ["PutCalendar", "CallCalendar", "DoubleCalendar"]);
        assert_eq!(
            named(TradeType::uses_tranche_entry),
            [
                "PutSpread",
                "CallSpread",
                "ShortPut",
                "ShortCall",
                "LongPut",
                "LongCall",
                "IronCondor",
                "IronCondor-Full",
                "IronFly",
                "ShortStrangle",
                "LongStrangle",
                "PutDebitSpread",
                "CallDebitSpread",
                "LongIronCondor-Full"
            ]
        );
        assert_eq!(named(TradeType::sells_only), ["ShortPut", "ShortCall", "ShortStrangle"]);
        assert_eq!(named(TradeType::buys_only), ["LongPut", "LongCall", "LongStrangle"]);
        assert_eq!(named(TradeType::is_debit_spread), ["PutDebitSpread", "CallDebitSpread", "LongIronCondor-Full"]);
        assert_eq!(named(TradeType::closes_longs_always), ["LongPut", "LongCall", "LongStrangle", "PutDebitSpread", "CallDebitSpread", "LongIronCondor-Full"]);
        assert_eq!(named(TradeType::is_single_long), ["LongPut", "LongCall"]);
        assert_eq!(named(TradeType::builds_put_verticals), ["PutSpread", "IronCondor", "IronCondor-Full", "IronFly", "PutDebitSpread", "LongIronCondor-Full"]);
        assert_eq!(
            named(TradeType::builds_call_verticals),
            ["CallSpread", "IronCondor", "IronCondor-Full", "IronFly", "CallDebitSpread", "LongIronCondor-Full"]
        );
    }

    #[test]
    fn the_sides_of_every_type_are_known() {
        // Every type the app enters has at least one side; only a custom trade has none fixed.
        for kind in TradeType::ALL {
            assert_eq!(kind.has_put_side() || kind.has_call_side(), kind != TradeType::Custom, "{kind:?}");
        }
        assert!(TradeType::PutCalendar.has_put_side() && !TradeType::PutCalendar.has_call_side());
        assert!(TradeType::DoubleCalendar.has_put_side() && TradeType::DoubleCalendar.has_call_side());
        // Verticals of one side are of a type that has that side.
        for kind in TradeType::ALL {
            assert!(!kind.builds_put_verticals() || kind.has_put_side(), "{kind:?}");
            assert!(!kind.builds_call_verticals() || kind.has_call_side(), "{kind:?}");
        }
    }

    #[test]
    fn names_that_are_no_type_are_debits_without_any_other_property() {
        assert!(is_debit("Something") && is_debit(""));
        assert!(!is_debit("PutSpread") && is_debit("PutCalendar"));
        assert!(!is("Something", TradeType::is_calendar) && is("DoubleCalendar", TradeType::is_calendar));
    }
}
