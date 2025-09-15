pub mod scheduler;
pub mod state;
pub mod switch;
pub mod task;

pub fn init() {
    scheduler::init();
}
