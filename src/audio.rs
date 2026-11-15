use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    path::Path,
    rc::Rc,
};
use vte4::gtk::{self, gio, glib, prelude::*};

#[derive(Clone, Default)]
pub struct Sounds(Option<Rc<Audio>>);

struct Audio {
    volume: f64,
    backends: Vec<(std::path::PathBuf, String)>,
    unavailable: RefCell<HashSet<usize>>,
    running: RefCell<HashSet<&'static str>>,
    clips: RefCell<HashMap<&'static str, gtk::MediaFile>>,
}

impl Sounds {
    pub fn new(enabled: bool, volume: f64) -> Self {
        Self(enabled.then(|| {
            let volume = volume.clamp(0.0, 1.0);
            let backends = [
                ("pw-play", format!("--volume={volume:.3}")),
                (
                    "paplay",
                    format!("--volume={}", (volume * 65536.0).round() as u32),
                ),
            ]
            .into_iter()
            .filter_map(|(name, volume)| Some((glib::find_program_in_path(name)?, volume)))
            .collect();
            Rc::new(Audio {
                volume,
                backends,
                unavailable: RefCell::new(HashSet::new()),
                running: RefCell::new(HashSet::new()),
                clips: RefCell::new(HashMap::new()),
            })
        }))
    }

    pub fn play(&self, name: &'static str) {
        if let Some(audio) = &self.0 {
            if audio.volume > 0.0 && !audio.running.borrow().contains(name) {
                audio.play(name, 0);
            }
        }
    }
}

impl Audio {
    fn play(self: &Rc<Self>, name: &'static str, start: usize) {
        let path = crate::theme::assets_path()
            .join("audio")
            .join(format!("{name}.wav"));
        if !path.is_file() {
            eprintln!("eDEX audio: missing {}", path.display());
            return;
        }
        for (index, (program, volume)) in self.backends.iter().enumerate().skip(start) {
            if self.unavailable.borrow().contains(&index) {
                continue;
            }
            let args: [&OsStr; 3] = [program.as_os_str(), OsStr::new(volume), path.as_os_str()];
            let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
            let Ok(child) = gio::Subprocess::newv(&args, flags) else {
                self.unavailable.borrow_mut().insert(index);
                continue;
            };
            self.running.borrow_mut().insert(name);
            let audio = self.clone();
            child.wait_check_async(None::<&gio::Cancellable>, move |result| {
                audio.running.borrow_mut().remove(name);
                if result.is_err() {
                    audio.unavailable.borrow_mut().insert(index);
                    audio.play(name, index + 1);
                }
            });
            return;
        }
        self.play_media(name, &path);
    }

    fn play_media(&self, name: &'static str, path: &Path) {
        let mut clips = self.clips.borrow_mut();
        let clip = clips.entry(name).or_insert_with(|| {
            let clip = gtk::MediaFile::for_filename(path);
            clip.set_volume(self.volume);
            clip.connect_notify_local(Some("error"), move |media, _| {
                if let Some(error) = media.error() {
                    eprintln!("eDEX audio ({name}): {error}. Check the user audio service or GTK GStreamer media backend.");
                }
            });
            clip
        });
        if clip.error().is_some() {
            return;
        }
        if clip.is_seekable() {
            clip.seek(0);
        }
        clip.play();
    }
}
