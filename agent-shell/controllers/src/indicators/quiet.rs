/// Quiet hours in minutes from midnight: `[from, to)`; the interval may wrap past midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    pub from: u16,
    pub to: u16,
}

impl QuietHours {
    pub fn contains(&self, minute: u16) -> bool {
        match self.from.cmp(&self.to) {
            std::cmp::Ordering::Equal => false,
            std::cmp::Ordering::Less => (self.from..self.to).contains(&minute),
            std::cmp::Ordering::Greater => minute >= self.from || minute < self.to,
        }
    }
}
