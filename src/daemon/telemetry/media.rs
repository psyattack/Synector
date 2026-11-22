use std::sync::Mutex;
use std::time::Instant;

#[cfg(windows)]
use super::audio::is_system_audio_playing;

// ---------------- ADVANCED MEDIA STATUS DETECTION ----------------

#[derive(Debug, Clone)]
pub struct SystemMediaStatus {
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub player_name: String,
}

static CACHED_MEDIA: Mutex<Option<(Instant, SystemMediaStatus)>> = Mutex::new(None);

pub fn get_system_media_status() -> SystemMediaStatus {
    if let Ok(lock) = CACHED_MEDIA.lock() {
        if let Some((time, ref status)) = *lock {
            if time.elapsed() < std::time::Duration::from_millis(200) {
                return status.clone();
            }
        }
    }

    #[cfg(windows)]
    {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, GetWindowThreadProcessId,
        };

        struct EnumContext {
            found: Option<SystemMediaStatus>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut EnumContext);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let len = GetWindowTextLengthW(hwnd);
            if !(3..=512).contains(&len) {
                return BOOL(1);
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let read_len = GetWindowTextW(hwnd, &mut buf);
            if read_len <= 0 {
                return BOOL(1);
            }
            let full_title = String::from_utf16_lossy(&buf[..read_len as usize]).trim().to_string();
            if full_title.is_empty() {
                return BOOL(1);
            }

            // Check YouTube
            if let Some(pos) = full_title.find(" - YouTube") {
                let track_part = full_title[..pos].trim();
                let is_nav = track_part.eq_ignore_ascii_case("youtube")
                    || track_part.eq_ignore_ascii_case("home")
                    || track_part.eq_ignore_ascii_case("subscriptions")
                    || track_part.eq_ignore_ascii_case("feed")
                    || track_part.is_empty();
                if !is_nav {
                    let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                        (a.trim().to_string(), t.trim().to_string())
                    } else {
                        ("YouTube".to_string(), track_part.to_string())
                    };
                    let is_active = is_system_audio_playing();
                    ctx.found = Some(SystemMediaStatus {
                        is_playing: is_active,
                        title,
                        artist,
                        player_name: "YouTube".to_string(),
                    });
                    return BOOL(0);
                }
            }

            // Check SoundCloud
            if let Some(pos) = full_title.find(" | SoundCloud") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("SoundCloud".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "SoundCloud".to_string(),
                });
                return BOOL(0);
            }

            // Check Yandex Music
            if full_title.contains("Яндекс Музыка") || full_title.contains("Яндекс.Музыка") {
                let clean = full_title
                    .replace("— Яндекс Музыка", "")
                    .replace("- Яндекс Музыка", "")
                    .replace("— Яндекс.Музыка", "")
                    .replace("- Яндекс.Музыка", "")
                    .trim()
                    .to_string();
                let (artist, title) = if let Some((a, t)) = clean.split_once(" — ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else if let Some((a, t)) = clean.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("Yandex Music".to_string(), clean)
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "Yandex Music".to_string(),
                });
                return BOOL(0);
            }

            // Check VLC
            if let Some(pos) = full_title.find(" - VLC media player") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("VLC Media Player".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "VLC".to_string(),
                });
                return BOOL(0);
            }

            // Check AIMP
            if let Some(pos) = full_title.find(" - AIMP") {
                let track_part = full_title[..pos].trim();
                let (artist, title) = if let Some((a, t)) = track_part.split_once(" - ") {
                    (a.trim().to_string(), t.trim().to_string())
                } else {
                    ("AIMP Player".to_string(), track_part.to_string())
                };
                let is_active = is_system_audio_playing();
                ctx.found = Some(SystemMediaStatus {
                    is_playing: is_active,
                    title,
                    artist,
                    player_name: "AIMP".to_string(),
                });
                return BOOL(0);
            }

            // Check Spotify: the window title alone is ambiguous, so confirm the owner process
            // with one image-path query instead of enumerating all processes per window.
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0 {
                let is_spotify_idle = full_title.eq_ignore_ascii_case("spotify") 
                    || full_title.eq_ignore_ascii_case("spotify free") 
                    || full_title.eq_ignore_ascii_case("spotify premium");
                
                if !is_spotify_idle && full_title.contains(" - ") {
                    let is_spotify = crate::daemon::watcher::ProcessWatcher::get_process_image_path(pid)
                        .as_deref()
                        .and_then(|path| std::path::Path::new(path).file_name().and_then(|name| name.to_str()))
                        .is_some_and(|name| name.eq_ignore_ascii_case("spotify.exe"));

                    if is_spotify {
                        let (artist, title) = if let Some((a, t)) = full_title.split_once(" - ") {
                            (a.trim().to_string(), t.trim().to_string())
                        } else {
                            ("Spotify".to_string(), full_title)
                        };
                        let is_active = is_system_audio_playing();
                        ctx.found = Some(SystemMediaStatus {
                            is_playing: is_active,
                            title,
                            artist,
                            player_name: "Spotify".to_string(),
                        });
                        return BOOL(0);
                    }
                }
            }

            BOOL(1)
        }

        let mut context = EnumContext { found: None };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut context as *mut _ as isize));
        }

        let status = context.found.unwrap_or_else(|| SystemMediaStatus {
            is_playing: false,
            title: "No Track Playing".to_string(),
            artist: "Synector Media".to_string(),
            player_name: "Media".to_string(),
        });

        if let Ok(mut lock) = CACHED_MEDIA.lock() {
            *lock = Some((Instant::now(), status.clone()));
        }
        status
    }
}
