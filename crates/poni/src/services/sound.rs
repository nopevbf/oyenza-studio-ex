//! System sounds for timers and reminders.

use crate::commands::Sound;
use windows::core::w;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};

pub fn play(s: Sound) {
    let alias = match s {
        Sound::Chime => w!("Notification.Reminder"),
        Sound::Tick => w!("SystemAsterisk"),

    };
    unsafe {
        if !PlaySoundW(alias, None, SND_ALIAS | SND_ASYNC | SND_NODEFAULT).as_bool() {
            let _ = PlaySoundW(w!("SystemNotification"), None, SND_ALIAS | SND_ASYNC);
        }
    }
}
