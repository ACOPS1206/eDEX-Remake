use sdl3::{
    AudioSubsystem, Error, Sdl,
    audio::{AudioDevice, AudioFormat, AudioSpec, AudioSpecWAV, AudioStreamOwner},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
    time::{Duration, Instant},
};
use vte4::gtk::glib;

#[derive(Clone, Default)]
pub struct Sounds(Option<Rc<Audio>>);

struct Audio {
    clips: RefCell<HashMap<&'static str, Clip>>,
    device: AudioDevice,
    subsystem: AudioSubsystem,
    _sdl: Sdl,
    volume: f32,
    last_play: Cell<Option<Instant>>,
}

struct Clip {
    wav: AudioSpecWAV,
    voices: Vec<AudioStreamOwner>,
}

impl Sounds {
    pub fn new(enabled: bool, volume: f64) -> Self {
        let volume = volume.clamp(0.0, 1.0) as f32;
        if !enabled || volume == 0.0 {
            return Self::default();
        }
        match Audio::new(volume) {
            Ok(audio) => Self(Some(Rc::new(audio))),
            Err(error) => {
                eprintln!("eDEX audio: {error}");
                Self::default()
            }
        }
    }

    pub fn play(&self, name: &'static str) {
        if let Some(audio) = &self.0 {
            if let Err(error) = audio.play(name) {
                eprintln!("eDEX audio ({name}): {error}");
            }
        }
    }
}

impl Audio {
    fn new(volume: f32) -> Result<Self, Error> {
        // GTK owns the application lifecycle, including termination from Cage.
        sdl3::hint::set("SDL_NO_SIGNAL_HANDLERS", "1");
        let sdl = sdl3::init()?;
        let subsystem = sdl.audio()?;
        let spec = AudioSpec::new(Some(44100), Some(2), Some(AudioFormat::F32LE));
        let device = subsystem.open_playback_device(&spec)?;
        if !device.pause() {
            return Err(sdl3::get_error());
        }
        Ok(Self {
            clips: RefCell::new(HashMap::new()),
            device,
            subsystem,
            _sdl: sdl,
            volume,
            last_play: Cell::new(None),
        })
    }

    fn play(self: &Rc<Self>, name: &'static str) -> Result<(), Error> {
        let mut clips = self.clips.borrow_mut();
        let clip = match clips.entry(name) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let path = crate::theme::assets_path()
                    .join("audio")
                    .join(format!("{name}.wav"));
                let wav = AudioSpecWAV::load_wav(path)?;
                entry.insert(Clip {
                    wav,
                    voices: vec![],
                })
            }
        };

        // A stream that is still playing can overlap the next instance of the same sound.
        let voice = match clip
            .voices
            .iter()
            .find(|voice| voice.queued_bytes().is_ok_and(|n| n == 0))
        {
            Some(voice) => voice,
            None => {
                let spec = AudioSpec::new(
                    Some(clip.wav.freq),
                    Some(i32::from(clip.wav.channels)),
                    Some(clip.wav.format),
                );
                let voice = self.subsystem.new_playback_stream(&spec, None)?;
                voice.set_gain(self.volume)?;
                self.device.bind_stream(&voice)?;
                clip.voices.push(voice);
                clip.voices.last().unwrap()
            }
        };
        voice.put_data(clip.wav.buffer())?;
        voice.flush()?;
        if self.last_play.replace(Some(Instant::now())).is_none() {
            if !self.device.resume() {
                self.last_play.set(None);
                return Err(sdl3::get_error());
            }
            let audio = Rc::downgrade(self);
            let mut drained = None;
            glib::timeout_add_local(Duration::from_millis(500), move || {
                let Some(audio) = audio.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let last_play = audio.last_play.get();
                if drained.is_some() && drained == last_play {
                    audio.device.pause();
                    audio.last_play.set(None);
                    return glib::ControlFlow::Break;
                }
                let all_drained = audio.clips.borrow().values().all(|clip| {
                    clip.voices
                        .iter()
                        .all(|voice| voice.queued_bytes().unwrap_or(0) == 0)
                });
                drained = last_play.filter(|_| all_drained);
                glib::ControlFlow::Continue
            });
        }
        Ok(())
    }
}
