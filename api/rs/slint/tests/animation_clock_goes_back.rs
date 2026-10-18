// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! A platform clock that reads earlier than its previous reading does not panic the event loop.
//! Windows' performance counter goes back a little on some machines.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

struct ClockPlatform;

impl slint::platform::Platform for ClockPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Err(slint::PlatformError::Other("this test needs no window".into()))
    }
    fn duration_since_start(&self) -> Duration {
        NOW.with(Cell::get)
    }
}

#[test]
fn an_event_loop_turn_after_the_clock_went_back_does_not_panic() {
    slint::platform::set_platform(Box::new(ClockPlatform)).unwrap();

    NOW.with(|now| now.set(Duration::from_millis(100)));
    slint::platform::update_timers_and_animations();

    NOW.with(|now| now.set(Duration::from_millis(99)));
    slint::platform::update_timers_and_animations();
}
