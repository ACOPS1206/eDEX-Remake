use sdl3::{
    AudioSubsystem, Error, Sdl,
    audio::{AudioDevice, AudioFormat, AudioSpec, AudioSpecWAV, AudioStreamOwner},
};
use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
};

#[derive(Clone, Default)]
pub struct Sounds(Option<Rc<Audio>>);

struct Audio {
    clips: RefCell<HashMap<&'static str, Clip>>,
    device: AudioDevice,
    subsystem: AudioSubsystem,
    _sdl: Sdl,
    volume: f32,
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
        // Mix short key clicks in smaller blocks when the backend supports it.
        sdl3::hint::set("SDL_AUDIO_DEVICE_SAMPLE_FRAMES", "256");
        let sdl = sdl3::init()?;
        let subsystem = sdl.audio()?;
        let spec = AudioSpec::new(Some(44100), Some(2), Some(AudioFormat::F32LE));
        // Keep the playback route active during gaps so the next click doesn't wake it.
        let device = subsystem.open_playback_device(&spec)?;
        let mut audio = Self {
            clips: RefCell::default(),
            device,
            subsystem,
            _sdl: sdl,
            volume,
        };
        match audio.load_clip("stdin") {
            Ok(clip) => {
                audio.clips.get_mut().insert("stdin", clip);
            }
            Err(error) => eprintln!("eDEX audio (stdin): {error}"),
        }
        Ok(audio)
    }

    fn load_clip(&self, name: &str) -> Result<Clip, Error> {
        let path = crate::theme::assets_path()
            .join("audio")
            .join(format!("{name}.wav"));
        let wav = AudioSpecWAV::load_wav(path)?;
        let voice = self.open_voice(&wav)?;
        Ok(Clip {
            wav,
            voices: vec![voice],
        })
    }

    fn open_voice(&self, wav: &AudioSpecWAV) -> Result<AudioStreamOwner, Error> {
        let spec = AudioSpec::new(
            Some(wav.freq),
            Some(i32::from(wav.channels)),
            Some(wav.format),
        );
        let voice = self.subsystem.new_playback_stream(&spec, None)?;
        voice.set_gain(self.volume)?;
        self.device.bind_stream(&voice)?;
        Ok(voice)
    }

    fn play(&self, name: &'static str) -> Result<(), Error> {
        let mut clips = self.clips.borrow_mut();
        let clip = match clips.entry(name) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(self.load_clip(name)?),
        };

        // A stream that is still playing can overlap the next instance of the same sound.
        let voice = match clip
            .voices
            .iter()
            .find(|voice| voice.queued_bytes().is_ok_and(|n| n == 0))
        {
            Some(voice) => voice,
            None => {
                clip.voices.push(self.open_voice(&clip.wav)?);
                clip.voices.last().unwrap()
            }
        };
        voice.put_data(clip.wav.buffer())?;
        voice.flush()
    }
}
