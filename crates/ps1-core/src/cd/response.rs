use crate::cd::CdController;

macro_rules! define_int_method {
    ($name:ident, $interrupt_level:literal) => {
        pub(super) fn $name(&mut self, response: &[u8]) {
            self.generate_response($interrupt_level, response);
        }
    };
}

impl CdController {
    fn generate_response(&mut self, interrupt_level: u8, response: &[u8]) {
        self.response_fifo.reset();
        for &byte in response {
            self.response_fifo.push(byte);
        }

        self.interrupts.flags |= interrupt_level;

        log::debug!("Set CD-ROM INT{interrupt_level} flag");
    }

    define_int_method!(int1, 1);
    define_int_method!(int2, 2);
    define_int_method!(int3, 3);
    define_int_method!(int4, 4);
    define_int_method!(int5, 5);
}
