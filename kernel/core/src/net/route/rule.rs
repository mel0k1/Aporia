use super::RouteTableId;

#[derive(Clone, Debug)]
pub(super) struct Rule {
    priority: RulePriority,
    action: RuleAction,
}

impl Rule {
    pub(super) const fn lookup(priority: u32, table: RouteTableId) -> Self {
        Self {
            priority: RulePriority::new(priority),
            action: RuleAction::Lookup(table),
        }
    }

    pub(super) const fn priority(&self) -> RulePriority {
        self.priority
    }

    pub(super) const fn action(&self) -> &RuleAction {
        &self.action
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RulePriority(u32);

impl RulePriority {
    pub(super) const fn new(priority: u32) -> Self {
        Self(priority)
    }
}

#[derive(Clone, Debug)]
pub(super) enum RuleAction {
    Lookup(RouteTableId),
    #[expect(dead_code)]
    Unreachable,
    #[expect(dead_code)]
    Prohibit,
    #[expect(dead_code)]
    Blackhole,
}
