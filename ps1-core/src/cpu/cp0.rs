use crate::cpu::Exception;
use crate::num::U32Ext;

const I_CACHE_LEN: usize = 4 * 1024;

type ICache = [u8; I_CACHE_LEN];

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
    pub kernel_mode: bool,
    pub interrupts_enabled: bool,
    pub kernel_mode_previous: bool,
    pub interrupts_enabled_previous: bool,
    pub kernel_mode_old: bool,
    pub interrupts_enabled_old: bool,
}

impl StatusRegister {
    fn new() -> Self {
        Self {
            boot_exception_vectors: true,
            isolate_cache: false,
            interrupt_mask: 0,
            kernel_mode: true,
            interrupts_enabled: false,
            kernel_mode_previous: true,
            interrupts_enabled_previous: false,
            kernel_mode_old: true,
            interrupts_enabled_old: false,
        }
    }

    fn read(&self) -> u32 {
        (u32::from(self.boot_exception_vectors) << 22)
            | (u32::from(self.isolate_cache) << 16)
            | (u32::from(self.interrupt_mask) << 8)
            | (u32::from(self.kernel_mode_old) << 5)
            | (u32::from(self.interrupts_enabled_old) << 4)
            | (u32::from(self.kernel_mode_previous) << 3)
            | (u32::from(self.interrupts_enabled_previous) << 2)
            | (u32::from(self.kernel_mode) << 1)
            | u32::from(self.interrupts_enabled)
    }

    fn write(&mut self, value: u32) {
        self.boot_exception_vectors = value.bit(22);
        self.isolate_cache = value.bit(16);
        self.interrupt_mask = (value >> 8) as u8;
        self.kernel_mode_old = value.bit(5);
        self.interrupts_enabled_old = value.bit(4);
        self.kernel_mode_previous = value.bit(3);
        self.interrupts_enabled_previous = value.bit(2);
        self.kernel_mode = value.bit(1);
        self.interrupts_enabled = value.bit(0);

        log::trace!("CP0 SR write ({value:08x}): {self:#?}");
    }

    fn push_exception_stack(&mut self) {
        self.kernel_mode_old = self.kernel_mode_previous;
        self.interrupts_enabled_old = self.interrupts_enabled_previous;

        self.kernel_mode_previous = self.kernel_mode;
        self.interrupts_enabled_previous = self.interrupts_enabled;

        self.kernel_mode = true;
        self.interrupts_enabled = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExceptionCode {
    #[default]
    Interrupt = 0,
    Syscall = 8,
}

impl ExceptionCode {
    fn from_bits(bits: u32) -> Self {
        match bits {
            0x00 => Self::Interrupt,
            0x08 => Self::Syscall,
            _ => {
                log::warn!("Unimplemented exception code: {bits:02X}");
                Self::Interrupt
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CauseRegister {
    pub branch_delay: bool,
    pub interrupts_pending: u8,
    pub exception_code: ExceptionCode,
}

impl CauseRegister {
    fn new() -> Self {
        Self {
            branch_delay: false,
            interrupts_pending: 0,
            exception_code: ExceptionCode::default(),
        }
    }

    fn read(&self) -> u32 {
        (u32::from(self.branch_delay) << 31)
            | (u32::from(self.interrupts_pending) << 8)
            | ((self.exception_code as u32) << 2)
    }

    fn write(&mut self, value: u32) {
        self.branch_delay = value.bit(31);
        self.interrupts_pending = (value >> 8) as u8;
        self.exception_code = ExceptionCode::from_bits((value >> 2) & 0x1F);
    }
}

#[derive(Debug, Clone)]
pub struct SystemControlCoprocessor {
    pub cache_control: CacheControl,
    pub status: StatusRegister,
    pub cause: CauseRegister,
    pub epc: u32,
    pub i_cache: Box<ICache>,
}

impl SystemControlCoprocessor {
    pub fn new() -> Self {
        Self {
            cache_control: CacheControl::new(),
            status: StatusRegister::new(),
            cause: CauseRegister::new(),
            epc: 0,
            i_cache: vec![0; I_CACHE_LEN].into_boxed_slice().try_into().unwrap(),
        }
    }

    pub fn read_register(&self, register: u32) -> u32 {
        match register {
            12 => self.status.read(),
            13 => self.cause.read(),
            14 => self.epc,
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
            13 => self.cause.write(value),
            _ => todo!("CP0 write {register} {value:08X}"),
        }
    }

    pub fn execute_operation(&mut self, operation: u32) {
        match operation & 0x3F {
            0x10 => {
                // RFE: Restore from exception
                self.status.kernel_mode = self.status.kernel_mode_previous;
                self.status.interrupts_enabled = self.status.interrupts_enabled_previous;

                self.status.kernel_mode_previous = self.status.kernel_mode_old;
                self.status.interrupts_enabled_previous = self.status.interrupts_enabled_old;
            }
            _ => todo!("CP0 operation {operation:06X}"),
        }
    }

    pub fn handle_exception(&mut self, exception: Exception, pc: u32, in_delay_slot: bool) {
        self.status.push_exception_stack();

        self.cause.branch_delay = in_delay_slot;
        self.cause.exception_code = exception.to_code();

        self.epc = if in_delay_slot {
            pc.wrapping_sub(4)
        } else {
            pc
        };
    }
}
