pub fn navigation_target(value: u32) -> u32 {
    value
}

pub fn navigation_caller() -> u32 {
    navigation_target(7)
}

pub fn deliberately_invalid( )->u32{
    "Task #208 expects a bounded diagnostic"
}
