pub enum Gp0Command {
    CpuToVramBlit,
}

pub enum Gp0CommandState {
    WaitingForCommand,
    WaitingForParameters { command: Gp0Command, remaining: u8 },
    CpuToVramCopy { x: u32, y: u32 },
}
