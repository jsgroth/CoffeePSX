use crate::num::U32Ext;

const I_CACHE_LEN: usize = 4 * 1024;
const D_CACHE_LEN: usize = 1024;

type ICache = [u8; I_CACHE_LEN];
type DCache = [u8; D_CACHE_LEN];

#[derive(Debug, Clone)]
pub struct CacheControl {
    pub i_cache_enabled: bool,
    pub d_cache_enabled: bool,
    pub scratchpad_enabled: bool,
}

impl CacheControl {
    fn new() -> Self {
        Self {
            i_cache_enabled: true,
            d_cache_enabled: true,
            scratchpad_enabled: true,
        }
    }

    pub fn write(&mut self, value: u32) {
        self.i_cache_enabled = value.bit(11);
        self.d_cache_enabled = value.bit(7);
        self.scratchpad_enabled = value.bit(3);

        log::trace!("Cache control write: {self:?}");
    }
}

#[derive(Debug, Clone)]
pub struct StatusRegister {
    pub boot_exception_vectors: bool,
    pub isolate_cache: bool,
    pub interrupt_mask: u8,
    pub user_mode: bool,
    pub interrupts_enabled: bool,
    pub user_mode_previous: bool,
    pub interrupts_enabled_previous: bool,
    pub user_mode_old: bool,
    pub interrupts_enabled_old: bool,
}

impl StatusRegister {
    fn new() -> Self {
        Self {
            boot_exception_vectors: true,
            isolate_cache: false,
            interrupt_mask: 0,
            user_mode: false,
            interrupts_enabled: false,
            user_mode_previous: false,
            interrupts_enabled_previous: false,
            user_mode_old: false,
            interrupts_enabled_old: false,
        }
    }

    fn read(&self) -> u32 {
        (u32::from(self.boot_exception_vectors) << 22)
            | (u32::from(self.isolate_cache) << 16)
            | (u32::from(self.interrupt_mask) << 8)
            | (u32::from(self.user_mode) << 5)
            | (u32::from(self.interrupts_enabled) << 4)
            | (u32::from(self.user_mode_previous) << 3)
            | (u32::from(self.interrupts_enabled_previous) << 2)
            | (u32::from(self.user_mode_old) << 1)
            | u32::from(self.interrupts_enabled_old)
    }

    fn write(&mut self, value: u32) {
        self.boot_exception_vectors = value.bit(22);
        self.isolate_cache = value.bit(16);
        self.interrupt_mask = (value >> 8) as u8;
        self.user_mode = value.bit(5);
        self.interrupts_enabled = value.bit(4);
        self.user_mode_previous = value.bit(3);
        self.interrupts_enabled_previous = value.bit(2);
        self.user_mode_old = value.bit(1);
        self.interrupts_enabled_old = value.bit(0);

        log::trace!("CP0 SR write ({value:08x}): {self:#?}");
    }
}

#[derive(Debug, Clone)]
pub struct SystemControlCoprocessor {
    pub cache_control: CacheControl,
    pub status: StatusRegister,
    pub i_cache: Box<ICache>,
    pub d_cache: Box<DCache>,
}

impl SystemControlCoprocessor {
    pub fn new() -> Self {
        Self {
            cache_control: CacheControl::new(),
            status: StatusRegister::new(),
            i_cache: vec![0; I_CACHE_LEN].into_boxed_slice().try_into().unwrap(),
            d_cache: vec![0; D_CACHE_LEN].into_boxed_slice().try_into().unwrap(),
        }
    }

    pub fn read_register(&self, register: u32) -> u32 {
        match register {
            12 => self.status.read(),
            _ => todo!("CP0 read {register}"),
        }
    }

    pub fn write_register(&mut self, register: u32, value: u32) {
        match register {
            3 => log::warn!("Unhandled CP0 breakpoint on execute write {value:08X}"),
            5 => log::warn!("Unhandled CP0 breakpoint on data access write {value:08X}"),
            6 => log::warn!("Unhandled CP0 JUMPDEST write {value:08X}"),
            7 => log::warn!("Unhandled CP0 breakpoint control write {value:08X}"),
            9 => log::warn!("Unhandled CP0 data access breakpoint mask write {value:08X}"),
            11 => log::warn!("Unhandled CP0 execute breakpoint mask write {value:08X}"),
            12 => self.status.write(value),
            13 => log::warn!("Unhandled CP0 Cause write {value:08X}"),
            _ => todo!("CP0 write {register} {value:08X}"),
        }
    }

    pub fn execute_operation(&mut self, operation: u32) {
        match operation & 0x3F {
            0x10 => {
                // RFE: Restore from exception
                self.status.user_mode = self.status.user_mode_previous;
                self.status.interrupts_enabled = self.status.interrupts_enabled_previous;

                self.status.user_mode_previous = self.status.user_mode_old;
                self.status.interrupts_enabled_previous = self.status.interrupts_enabled_old;
            }
            _ => todo!("CP0 operation {operation:06X}"),
        }
    }
}
