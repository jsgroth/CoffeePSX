pub trait U32Ext {
    fn bit(self, i: u8) -> bool;

    fn sign_bit(self) -> bool;
}

impl U32Ext for u32 {
    fn bit(self, i: u8) -> bool {
        self & (1 << i) != 0
    }

    fn sign_bit(self) -> bool {
        self.bit(31)
    }
}