use std::fmt::Display;

const SAFE_MODE: bool = false;

pub struct Sensitive<T: Display>(pub T);
impl<T: Display> Display for Sensitive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if SAFE_MODE {
            write!(f, "[REDACTED]")
        } else {
            write!(f, "{value}", value = self.0)
        }
    }
}
