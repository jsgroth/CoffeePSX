use crate::scheduler::heap::UpdateableMinHeap;
use bincode::{Decode, Encode};
use std::cmp::Ordering;

mod heap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SchedulerEventType {
    VBlank,
    SpuAndCdClock,
    Timer0Irq,
    Timer1Irq,
    Timer2Irq,
    Sio0Irq,
    Sio0Tx,
    Sio1Irq,
    Sio1Tx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct SchedulerEvent {
    pub event_type: SchedulerEventType,
    pub cpu_cycles: u64,
}

impl SchedulerEvent {
    pub fn vblank(cpu_cycles: u64) -> Self {
        Self { event_type: SchedulerEventType::VBlank, cpu_cycles }
    }

    pub fn spu_and_cd_clock(cpu_cycles: u64) -> Self {
        Self { event_type: SchedulerEventType::SpuAndCdClock, cpu_cycles }
    }
}

impl Default for SchedulerEvent {
    fn default() -> Self {
        Self { event_type: SchedulerEventType::VBlank, cpu_cycles: u64::MAX }
    }
}

impl PartialOrd for SchedulerEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchedulerEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cpu_cycles.cmp(&other.cpu_cycles)
    }
}

type SchedulerHeap = UpdateableMinHeap<SchedulerEvent, 20>;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Scheduler {
    cpu_cycle_counter: u64,
    heap: SchedulerHeap,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { cpu_cycle_counter: 0, heap: SchedulerHeap::new() }
    }

    pub fn cpu_cycle_counter(&self) -> u64 {
        self.cpu_cycle_counter
    }

    pub fn increment_cpu_cycles(&mut self, cpu_cycles: u64) {
        self.cpu_cycle_counter += cpu_cycles;
    }

    pub fn is_event_ready(&self) -> bool {
        self.cpu_cycle_counter >= self.heap.peek().cpu_cycles
    }

    pub fn update_or_push_event(&mut self, event: SchedulerEvent) {
        log::debug!(
            "Scheduled event of type {:?} at cycles {}, current {}",
            event.event_type,
            event.cpu_cycles,
            self.cpu_cycle_counter
        );

        self.heap
            .update_or_push(event, |existing_event| event.event_type == existing_event.event_type);
    }

    pub fn remove_event(&mut self, event_type: SchedulerEventType) {
        self.heap.remove_one(|event| event.event_type == event_type);
    }

    pub fn pop_ready_event(&mut self) -> Option<SchedulerEvent> {
        self.is_event_ready().then(|| self.heap.pop())
    }
}
