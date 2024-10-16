macro_rules! impl_ext_trait {
    ($name:ident, $t:ty $(, $sign_bit:ident)?) => {
        pub trait $name {
            fn bit(self, i: u8) -> bool;

            $(fn $sign_bit(self) -> bool;)?
        }

        impl $name for $t {
            #[inline(always)]
            fn bit(self, i: u8) -> bool {
                self & (1 << i) != 0
            }

            $(
                #[inline(always)]
                fn $sign_bit(self) -> bool {
                    self.bit((<$t>::BITS - 1) as u8)
                }
            )?
        }
    };
}

impl_ext_trait!(U8Ext, u8);
impl_ext_trait!(U16Ext, u16);
impl_ext_trait!(U32Ext, u32, sign_bit);
impl_ext_trait!(I16Ext, i16);
