use std::fmt::{self, LowerHex};

pub struct Hex<T: LowerHex>(pub T);

impl<T: LowerHex> fmt::Debug for Hex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("0x")?;
        LowerHex::fmt(&self.0, f)
    }
}
