use super::*;

const INTERRUPT_TYPE: InterruptType = InterruptType::Timer0;

#[test]
fn timer_reset_at_max_no_irq() {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    let mut interrupt_registers = InterruptRegisters::new();

    let mut counter: u16 = 0;
    let mut overflowed = false;
    for _ in 0..100000 {
        let clocks: u16 = rand::random();
        let (new_counter, add_overflowed) = counter.overflowing_add(clocks);
        counter = new_counter;
        overflowed |= add_overflowed || counter == 0xFFFF;

        timer.clock(clocks.into(), &mut interrupt_registers);

        assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
        assert_eq!(timer.counter, counter);
        assert_eq!(timer.reached_max, overflowed);
    }
}

#[test]
fn timer_reset_at_max_with_max_irq() {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    timer.irq_at_max = true;
    timer.irq_repeat_mode = IrqRepeatMode::Repeat;

    let mut interrupt_registers = InterruptRegisters::new();

    let mut counter: u16 = 0;
    for _ in 0..100000 {
        let clocks: u64 = rand::random();
        let reached_max = (counter != 0xFFFF && clocks >= u64::from(0xFFFF - counter))
            || (counter == 0xFFFF && clocks >= 0x10000);

        counter = counter.wrapping_add(clocks as u16);

        timer.clock(clocks.into(), &mut interrupt_registers);

        assert_eq!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE), reached_max);
        assert_eq!(timer.counter, counter);
        assert_eq!(timer.reached_max, reached_max);

        timer.reached_max = false;
        interrupt_registers.write_interrupt_status(0);
    }
}

#[test]
fn timer_reset_at_max_with_target_irq() {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    let target: u16 = rand::random();
    timer.target = target;
    timer.irq_at_target = true;
    timer.irq_repeat_mode = IrqRepeatMode::Repeat;

    let mut interrupt_registers = InterruptRegisters::new();

    let mut counter: u16 = 0;
    for _ in 0..5000 {
        let clocks = rand::random::<u64>() & 0x1FFFF;
        let reached_target =
            clocks >= 0x10000 || (1..=clocks).any(|n| counter.wrapping_add(n as u16) == target);

        counter = counter.wrapping_add(clocks as u16);

        timer.clock(clocks.into(), &mut interrupt_registers);

        assert_eq!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE), reached_target);
        assert_eq!(timer.counter, counter);
        assert_eq!(timer.reached_target, reached_target);

        timer.reached_target = false;
        interrupt_registers.write_interrupt_status(0);
    }
}

#[test]
fn timer_reset_at_max_toggle_irq() {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    timer.irq_at_max = true;
    timer.irq_repeat_mode = IrqRepeatMode::Repeat;
    timer.irq_pulse_mode = IrqPulseMode::Toggle;

    let mut interrupt_registers = InterruptRegisters::new();

    timer.clock(0xFFFE, &mut interrupt_registers);
    assert_eq!(timer.counter, 0xFFFE);
    assert!(!timer.reached_max);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(1, &mut interrupt_registers);
    assert_eq!(timer.counter, 0xFFFF);
    assert!(timer.reached_max);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(timer.irq);

    timer.reached_max = false;
    interrupt_registers.write_interrupt_status(0);

    timer.clock(1, &mut interrupt_registers);
    assert_eq!(timer.counter, 0);
    assert!(!timer.reached_max);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(timer.irq);

    timer.clock(0x10004, &mut interrupt_registers);
    assert_eq!(timer.counter, 4);
    assert!(timer.reached_max);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.reached_max = false;

    timer.clock(0x20000, &mut interrupt_registers);
    assert_eq!(timer.counter, 4);
    assert!(timer.reached_max);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);
}

fn timer_reset_at_target(test_irq: bool) {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    timer.reset_at_target = true;
    timer.target = 5000;
    timer.irq_at_target = test_irq;
    timer.irq_repeat_mode = IrqRepeatMode::Repeat;

    let mut interrupt_registers = InterruptRegisters::new();

    timer.clock(2500, &mut interrupt_registers);
    assert_eq!(timer.counter, 2500);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(100, &mut interrupt_registers);
    assert_eq!(timer.counter, 2600);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(2400, &mut interrupt_registers);
    assert_eq!(timer.counter, 5000);
    assert!(timer.reached_target);
    assert_eq!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE), test_irq);
    assert!(!timer.irq);

    timer.reached_target = false;
    interrupt_registers.write_interrupt_status(0);

    timer.clock(1, &mut interrupt_registers);
    assert_eq!(timer.counter, 0);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(5005, &mut interrupt_registers);
    assert_eq!(timer.counter, 4);
    assert!(timer.reached_target);
    assert_eq!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE), test_irq);
    assert!(!timer.irq);
}

#[test]
fn timer_reset_at_target_no_irq() {
    timer_reset_at_target(false);
}

#[test]
fn timer_reset_at_target_with_irq() {
    timer_reset_at_target(true);
}

#[test]
fn timer_reset_at_target_toggle_irq() {
    let mut timer = SystemTimer::new(INTERRUPT_TYPE);
    timer.reset_at_target = true;
    timer.target = 5000;
    timer.irq_at_target = true;
    timer.irq_repeat_mode = IrqRepeatMode::Repeat;
    timer.irq_pulse_mode = IrqPulseMode::Toggle;

    let mut interrupt_registers = InterruptRegisters::new();

    timer.clock(2500, &mut interrupt_registers);
    assert_eq!(timer.counter, 2500);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(100, &mut interrupt_registers);
    assert_eq!(timer.counter, 2600);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.clock(2400, &mut interrupt_registers);
    assert_eq!(timer.counter, 5000);
    assert!(timer.reached_target);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(timer.irq);

    timer.reached_target = false;
    interrupt_registers.write_interrupt_status(0);

    timer.clock(1, &mut interrupt_registers);
    assert_eq!(timer.counter, 0);
    assert!(!timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));

    timer.clock(5005, &mut interrupt_registers);
    assert_eq!(timer.counter, 4);
    assert!(timer.reached_target);
    assert!(!interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);

    timer.reached_target = false;

    timer.clock(5002, &mut interrupt_registers);
    assert_eq!(timer.counter, 5);
    assert!(timer.reached_target);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(timer.irq);

    timer.reached_target = false;
    interrupt_registers.write_interrupt_status(0);

    timer.clock(2 * 5001, &mut interrupt_registers);
    assert_eq!(timer.counter, 5);
    assert!(timer.reached_target);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(timer.irq);

    timer.reached_target = false;
    interrupt_registers.write_interrupt_status(0);

    timer.clock(3 * 5001, &mut interrupt_registers);
    assert_eq!(timer.counter, 5);
    assert!(timer.reached_target);
    assert!(interrupt_registers.read_interrupt_flag(INTERRUPT_TYPE));
    assert!(!timer.irq);
}
