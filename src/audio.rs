use sdl3::{
    AudioSubsystem, Error, Sdl,
    audio::{AudioDevice, AudioFormat, AudioSpec, AudioSpecWAV, AudioStreamOwner},
};
use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
    time::Duration,
};
use gtk4::glib;

// A quarter-second of 44.1 kHz stereo f32 silence keeps PipeWire/PulseAudio
// from suspending the output between key presses. It is never audible.
const SILENCE: [u8; 88_200] = [0; 88_200];

#[derive(Clone, Default)]
pub struct Sounds(Option<Rc<Audio>>);

struct Audio {
    clips: RefCell<HashMap<&'static str, Clip>>,
    keep_alive_stream: AudioStreamOwner,
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
            Ok(audio) => {
                let audio = Rc::new(audio);
                let weak = Rc::downgrade(&audio);
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    let Some(audio) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if let Err(error) = audio.refill_silence() {
                        eprintln!("eDEX audio keep-alive: {error}");
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
                Self(Some(audio))
            }
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
        // Binding and feeding a stream now, rather than on the first keystroke,
        // gives the native audio backend time to start its playback route.
        let keep_alive_stream = subsystem.new_playback_stream(&spec, None)?;
        device.bind_stream(&keep_alive_stream)?;
        keep_alive_stream.put_data(&SILENCE)?;
        device.resume();
        let mut audio = Self {
            clips: RefCell::default(),
            keep_alive_stream,
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

    fn refill_silence(&self) -> Result<(), Error> {
        if self.keep_alive_stream.queued_bytes()? < SILENCE.len() as i32 {
            self.keep_alive_stream.put_data(&SILENCE)?;
        }
        Ok(())
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
