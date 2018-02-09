// Copyright 2016-2018 Mateusz Sieczko and other GilRs Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use super::FfDevice;
use ev::{RawEvent, RawEventType};
use ev::AxisInfo;
use gamepad::{self, GamepadImplExt, PlatformError, PowerInfo, Status};

use uuid::Uuid;
use winapi::winerror::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};
use winapi::xinput::{self as xi, XINPUT_BATTERY_INFORMATION as XBatteryInfo,
                     XINPUT_GAMEPAD as XGamepad, XINPUT_STATE as XState, XINPUT_GAMEPAD_A,
                     XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_DPAD_DOWN,
                     XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
                     XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
                     XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB,
                     XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y};
use xinput;

use std::{mem, thread, u16, u32};
use std::collections::VecDeque;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

// Chosen by dice roll ;)
const EVENT_THREAD_SLEEP_TIME: u64 = 10;
const ITERATIONS_TO_CHECK_IF_CONNECTED: u64 = 100;

#[derive(Debug)]
pub struct Gilrs {
    gamepads: [gamepad::Gamepad; 4],
    rx: Receiver<RawEvent>,
    not_observed: gamepad::Gamepad,
    additional_events: VecDeque<RawEvent>,
}

impl Gilrs {
    pub(crate) fn new() -> Result<Self, PlatformError> {
        let gamepads = [
            gamepad_new(0),
            gamepad_new(1),
            gamepad_new(2),
            gamepad_new(3),
        ];

        let connected = [
            gamepads[0].is_connected(),
            gamepads[1].is_connected(),
            gamepads[2].is_connected(),
            gamepads[3].is_connected(),
        ];

        let additional_events = connected
            .iter()
            .enumerate()
            .filter(|&(_, &con)| con)
            .map(|(i, _)| RawEvent::new(i, RawEventType::Connected))
            .collect();

        unsafe { xinput::XInputEnable(1) };
        let (tx, rx) = mpsc::channel();
        Self::spawn_thread(tx, connected);

        Ok(Gilrs {
            gamepads,
            rx,
            not_observed: gamepad::Gamepad::from_inner_status(Gamepad::none(), Status::NotObserved),
            additional_events,
        })
    }

    pub(crate) fn next_event(&mut self) -> Option<RawEvent> {
        if let Some(event) = self.additional_events.pop_front() {
            Some(event)
        } else {
            self.rx.try_recv().ok()
        }
    }

    pub fn gamepad(&self, id: usize) -> &gamepad::Gamepad {
        self.gamepads.get(id).unwrap_or(&self.not_observed)
    }

    pub fn gamepad_mut(&mut self, id: usize) -> &mut gamepad::Gamepad {
        self.gamepads.get_mut(id).unwrap_or(&mut self.not_observed)
    }

    pub fn last_gamepad_hint(&self) -> usize {
        self.gamepads.len()
    }

    fn spawn_thread(tx: Sender<RawEvent>, connected: [bool; 4]) {
        thread::spawn(move || unsafe {
            let mut prev_state = mem::zeroed::<XState>();
            let mut state = mem::zeroed::<XState>();
            let mut connected = connected;
            let mut counter = 0;

            loop {
                for id in 0..4 {
                    if *connected.get_unchecked(id)
                        || counter % ITERATIONS_TO_CHECK_IF_CONNECTED == 0
                    {
                        let val = xinput::XInputGetState(id as u32, &mut state);

                        if val == ERROR_SUCCESS {
                            if !connected.get_unchecked(id) {
                                *connected.get_unchecked_mut(id) = true;
                                let _ = tx.send(RawEvent::new(id, RawEventType::Connected));
                            }

                            if state.dwPacketNumber != prev_state.dwPacketNumber {
                                Self::compare_state(id, &state.Gamepad, &prev_state.Gamepad, &tx);
                                prev_state = state;
                            }
                        } else if val == ERROR_DEVICE_NOT_CONNECTED && *connected.get_unchecked(id)
                        {
                            *connected.get_unchecked_mut(id) = false;
                            let _ = tx.send(RawEvent::new(id, RawEventType::Disconnected));
                        }
                    }
                }

                counter = counter.wrapping_add(1);
                thread::sleep(Duration::from_millis(EVENT_THREAD_SLEEP_TIME));
            }
        });
    }

    fn compare_state(id: usize, g: &XGamepad, pg: &XGamepad, tx: &Sender<RawEvent>) {
        if g.bLeftTrigger != pg.bLeftTrigger {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.bLeftTrigger as i32, native_ev_codes::AXIS_LT2),
            ));
        }
        if g.bRightTrigger != pg.bRightTrigger {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.bRightTrigger as i32, native_ev_codes::AXIS_RT2),
            ));
        }
        if g.sThumbLX != pg.sThumbLX {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.sThumbLX as i32, native_ev_codes::AXIS_LSTICKX),
            ));
        }
        if g.sThumbLY != pg.sThumbLY {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.sThumbLY as i32, native_ev_codes::AXIS_LSTICKY),
            ));
        }
        if g.sThumbRX != pg.sThumbRX {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.sThumbRX as i32, native_ev_codes::AXIS_RSTICKX),
            ));
        }
        if g.sThumbRY != pg.sThumbRY {
            let _ = tx.send(RawEvent::new(
                id,
                RawEventType::AxisValueChanged(g.sThumbRY as i32, native_ev_codes::AXIS_RSTICKY),
            ));
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_UP) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_UP != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_DPAD_UP),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_DPAD_UP),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_DOWN) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_DOWN != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_DPAD_DOWN),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_DPAD_DOWN),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_LEFT) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_LEFT != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_DPAD_LEFT),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_DPAD_LEFT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_RIGHT) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_DPAD_RIGHT),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_DPAD_RIGHT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_START) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_START != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_START),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_START),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_BACK) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_BACK != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_SELECT),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_SELECT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_LEFT_THUMB) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_LEFT_THUMB != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_LTHUMB),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_LTHUMB),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_RIGHT_THUMB) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_RTHUMB),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_RTHUMB),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_LEFT_SHOULDER) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_LT),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_LT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_RIGHT_SHOULDER) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_RT),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_RT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_A) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_A != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_SOUTH),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_SOUTH),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_B) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_B != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_EAST),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_EAST),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_X) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_X != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_WEST),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_WEST),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_Y) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_Y != 0 {
                true => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonPressed(native_ev_codes::BTN_NORTH),
                )),
                false => tx.send(RawEvent::new(
                    id,
                    RawEventType::ButtonReleased(native_ev_codes::BTN_NORTH),
                )),
            };
        }
    }
}

#[derive(Debug)]
pub struct Gamepad {
    uuid: Uuid,
    id: u32,
}

impl Gamepad {
    fn none() -> Self {
        Gamepad {
            uuid: Uuid::nil(),
            id: u32::MAX,
        }
    }

    pub fn name(&self) -> &str {
        "Xbox Controller"
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn power_info(&self) -> PowerInfo {
        unsafe {
            let mut binfo = mem::uninitialized::<XBatteryInfo>();
            if xinput::XInputGetBatteryInformation(self.id, xi::BATTERY_DEVTYPE_GAMEPAD, &mut binfo)
                == ERROR_SUCCESS
            {
                match binfo.BatteryType {
                    xi::BATTERY_TYPE_WIRED => PowerInfo::Wired,
                    xi::BATTERY_TYPE_ALKALINE | xi::BATTERY_TYPE_NIMH => {
                        let lvl = match binfo.BatteryLevel {
                            xi::BATTERY_LEVEL_EMPTY => 0,
                            xi::BATTERY_LEVEL_LOW => 33,
                            xi::BATTERY_LEVEL_MEDIUM => 67,
                            xi::BATTERY_LEVEL_FULL => 100,
                            _ => unreachable!(),
                        };
                        if lvl == 100 {
                            PowerInfo::Charged
                        } else {
                            PowerInfo::Discharging(lvl)
                        }
                    }
                    _ => PowerInfo::Unknown,
                }
            } else {
                PowerInfo::Unknown
            }
        }
    }

    pub fn is_ff_supported(&self) -> bool {
        true
    }

    pub fn ff_device(&self) -> Option<FfDevice> {
        Some(FfDevice::new(self.id))
    }

    pub fn buttons(&self) -> &[EvCode] {
        &native_ev_codes::BUTTONS
    }

    pub fn axes(&self) -> &[EvCode] {
        &native_ev_codes::AXES
    }

    pub(crate) fn axis_info(&self, nec: EvCode) -> Option<&AxisInfo> {
        native_ev_codes::AXES_INFO
            .get(nec.0 as usize)
            .and_then(|o| o.as_ref())
    }
}

#[inline(always)]
fn is_mask_eq(l: u16, r: u16, mask: u16) -> bool {
    (l & mask != 0) == (r & mask != 0)
}

fn gamepad_new(id: u32) -> gamepad::Gamepad {
    let gamepad = Gamepad {
        uuid: Uuid::nil(),
        id,
    };

    let status = unsafe {
        let mut state = mem::zeroed::<XState>();
        if xinput::XInputGetState(id, &mut state) == ERROR_SUCCESS {
            Status::Connected
        } else {
            Status::NotObserved
        }
    };

    gamepad::Gamepad::from_inner_status(gamepad, status)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EvCode(u8);

impl Display for EvCode {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        self.0.fmt(f)
    }
}

pub mod native_ev_codes {
    use std::i16::{MAX as I16_MAX, MIN as I16_MIN};
    use std::u8::{MAX as U8_MAX, MIN as U8_MIN};

    use winapi::xinput::{XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE, XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE,
                         XINPUT_GAMEPAD_TRIGGER_THRESHOLD};

    use super::EvCode;
    use ev::AxisInfo;

    pub const AXIS_LSTICKX: EvCode = EvCode(0);
    pub const AXIS_LSTICKY: EvCode = EvCode(1);
    pub const AXIS_LEFTZ: EvCode = EvCode(2);
    pub const AXIS_RSTICKX: EvCode = EvCode(3);
    pub const AXIS_RSTICKY: EvCode = EvCode(4);
    pub const AXIS_RIGHTZ: EvCode = EvCode(5);
    pub const AXIS_DPADX: EvCode = EvCode(6);
    pub const AXIS_DPADY: EvCode = EvCode(7);
    pub const AXIS_RT: EvCode = EvCode(8);
    pub const AXIS_LT: EvCode = EvCode(9);
    pub const AXIS_RT2: EvCode = EvCode(10);
    pub const AXIS_LT2: EvCode = EvCode(11);

    pub const BTN_SOUTH: EvCode = EvCode(12);
    pub const BTN_EAST: EvCode = EvCode(13);
    pub const BTN_C: EvCode = EvCode(14);
    pub const BTN_NORTH: EvCode = EvCode(15);
    pub const BTN_WEST: EvCode = EvCode(16);
    pub const BTN_Z: EvCode = EvCode(17);
    pub const BTN_LT: EvCode = EvCode(18);
    pub const BTN_RT: EvCode = EvCode(19);
    pub const BTN_LT2: EvCode = EvCode(20);
    pub const BTN_RT2: EvCode = EvCode(21);
    pub const BTN_SELECT: EvCode = EvCode(22);
    pub const BTN_START: EvCode = EvCode(23);
    pub const BTN_MODE: EvCode = EvCode(24);
    pub const BTN_LTHUMB: EvCode = EvCode(25);
    pub const BTN_RTHUMB: EvCode = EvCode(26);

    pub const BTN_DPAD_UP: EvCode = EvCode(27);
    pub const BTN_DPAD_DOWN: EvCode = EvCode(28);
    pub const BTN_DPAD_LEFT: EvCode = EvCode(29);
    pub const BTN_DPAD_RIGHT: EvCode = EvCode(30);

    pub(super) static BUTTONS: [EvCode; 15] = [
        BTN_SOUTH,
        BTN_EAST,
        BTN_NORTH,
        BTN_WEST,
        BTN_LT,
        BTN_RT,
        BTN_SELECT,
        BTN_START,
        BTN_MODE,
        BTN_LTHUMB,
        BTN_RTHUMB,
        BTN_DPAD_UP,
        BTN_DPAD_DOWN,
        BTN_DPAD_LEFT,
        BTN_DPAD_RIGHT,
    ];

    pub(super) static AXES: [EvCode; 6] = [
        AXIS_LSTICKX,
        AXIS_LSTICKY,
        AXIS_RSTICKX,
        AXIS_RSTICKY,
        AXIS_RT2,
        AXIS_LT2,
    ];

    pub(super) static AXES_INFO: [Option<AxisInfo>; 12] = [
        // LeftStickX
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as u32,
        }),
        // LeftStickY
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as u32,
        }),
        // LeftZ
        None,
        // RightStickX
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE as u32,
        }),
        // RightStickY
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE as u32,
        }),
        // RightZ
        None,
        // DPadX
        None,
        // DPadY
        None,
        // RightTrigger
        None,
        // LeftTrigger
        None,
        // RightTrigger2
        Some(AxisInfo {
            min: U8_MIN as i32,
            max: U8_MAX as i32,
            deadzone: XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u32,
        }),
        // LeftTrigger2
        Some(AxisInfo {
            min: U8_MIN as i32,
            max: U8_MAX as i32,
            deadzone: XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u32,
        }),
    ];
}
