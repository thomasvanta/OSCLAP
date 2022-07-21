use crossbeam_channel::{Receiver, Sender};
use nih_plug::debug::*;
use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;
use rosc::{OscMessage, OscMidiMessage, OscPacket, OscType};
use rubato::{Resampler, FftFixedIn};
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

mod editor;
mod param_view;

struct DawOut {
    params: Arc<DawOutParams>,
    sender: Option<Sender<OscChannelMessageType>>,
    editor_state: Arc<ViziaState>,
    p1_dirty: Arc<AtomicBool>,
    p2_dirty: Arc<AtomicBool>,
    p3_dirty: Arc<AtomicBool>,
    p4_dirty: Arc<AtomicBool>,
    p5_dirty: Arc<AtomicBool>,
    p6_dirty: Arc<AtomicBool>,
    p7_dirty: Arc<AtomicBool>,
    p8_dirty: Arc<AtomicBool>,
}

impl Default for DawOut {
    fn default() -> Self {
        let p1_dirty = Arc::new(AtomicBool::new(false));
        let p2_dirty = Arc::new(AtomicBool::new(false));
        let p3_dirty = Arc::new(AtomicBool::new(false));
        let p4_dirty = Arc::new(AtomicBool::new(false));
        let p5_dirty = Arc::new(AtomicBool::new(false));
        let p6_dirty = Arc::new(AtomicBool::new(false));
        let p7_dirty = Arc::new(AtomicBool::new(false));
        let p8_dirty = Arc::new(AtomicBool::new(false));

        Self {
            params: Arc::new(DawOutParams::new(
                p1_dirty.clone(),
                p2_dirty.clone(),
                p3_dirty.clone(),
                p4_dirty.clone(),
                p5_dirty.clone(),
                p6_dirty.clone(),
                p7_dirty.clone(),
                p8_dirty.clone(),
            )),
            sender: None,
            editor_state: editor::default_state(),
            p1_dirty,
            p2_dirty,
            p3_dirty,
            p4_dirty,
            p5_dirty,
            p6_dirty,
            p7_dirty,
            p8_dirty,
        }
    }
}

impl Drop for DawOut {
    fn drop(&mut self) {
        if let Some(sender) = &self.sender {
            sender.send(OscChannelMessageType::Exit).unwrap();
        }
    }
}

struct OscChannel {
    sender: Sender<OscChannelMessageType>,
    receiver: Receiver<OscChannelMessageType>,
}

impl Default for OscChannel {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(65_536);
        Self { sender, receiver }
    }
}

struct OscParamType {
    address_base: String,
    name: String,
    value: f32,
}

struct OscNoteType {
    address_base: String,
    channel: u8,
    note: u8,
    velocity: f32,
}

struct OscAudioType {
    address_base: String,
    value: f32,
}

//TODO: osc server address/port update?
enum OscChannelMessageType {
    Exit,
    Param(OscParamType),
    NoteOn(OscNoteType),
    NoteOff(OscNoteType),
    Audio(OscAudioType),
}

#[derive(Params)]
pub struct DawOutParams {
    //Persisted Settings
    #[persist = "osc_server_address"]
    osc_server_address: RwLock<String>,
    #[persist = "osc_server_port"]
    osc_server_port: RwLock<u16>,
    #[persist = "osc_address_base"]
    osc_address_base: RwLock<String>,

    //Setting Flags
    #[id = "flag_send_midi"]
    flag_send_midi: BoolParam,
    #[id = "flag_send_audio"]
    flag_send_audio: BoolParam,

    //Exposed Params
    #[id = "param1"]
    param1: FloatParam,
    #[id = "param2"]
    param2: FloatParam,
    #[id = "param3"]
    param3: FloatParam,
    #[id = "param4"]
    param4: FloatParam,
    #[id = "param5"]
    param5: FloatParam,
    #[id = "param6"]
    param6: FloatParam,
    #[id = "param7"]
    param7: FloatParam,
    #[id = "param8"]
    param8: FloatParam,
}

impl DawOutParams {
    #[allow(clippy::derivable_impls)]
    fn new(
        p1_dirty: Arc<AtomicBool>,
        p2_dirty: Arc<AtomicBool>,
        p3_dirty: Arc<AtomicBool>,
        p4_dirty: Arc<AtomicBool>,
        p5_dirty: Arc<AtomicBool>,
        p6_dirty: Arc<AtomicBool>,
        p7_dirty: Arc<AtomicBool>,
        p8_dirty: Arc<AtomicBool>,
    ) -> Self {
        Self {
            osc_server_address: RwLock::new("127.0.0.1".to_string()),
            osc_server_port: RwLock::new(9000),
            osc_address_base: RwLock::new("daw-out".to_string()),
            flag_send_midi: BoolParam::new("flag_send_midi", true).hide().non_automatable(),
            flag_send_audio: BoolParam::new("flag_send_audio", false).hide().non_automatable(),
            param1: FloatParam::new("param1", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p1_dirty.store(true, Ordering::Release))),
            param2: FloatParam::new("param2", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p2_dirty.store(true, Ordering::Release))),
            param3: FloatParam::new("param3", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p3_dirty.store(true, Ordering::Release))),
            param4: FloatParam::new("param4", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p4_dirty.store(true, Ordering::Release))),
            param5: FloatParam::new("param5", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p5_dirty.store(true, Ordering::Release))),
            param6: FloatParam::new("param6", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p6_dirty.store(true, Ordering::Release))),
            param7: FloatParam::new("param7", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p7_dirty.store(true, Ordering::Release))),
            param8: FloatParam::new("param8", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.01)
                .with_callback(Arc::new(move |_x| p8_dirty.store(true, Ordering::Release))),
        }
    }
}

impl Plugin for DawOut {
    const NAME: &'static str = "DAW Out";
    const VENDOR: &'static str = "gamingrobot";
    const URL: &'static str = "https://github.com/gamingrobot/daw-out";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    const HARD_REALTIME_ONLY: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        nih_trace!("Params Called");
        self.params.clone() as Arc<dyn Params>
    }

    fn editor(&self) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.editor_state.clone(),
        )
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext,
    ) -> bool {
        nih_trace!("Initialize Called");

        if buffer_config.process_mode != ProcessMode::Realtime {
            nih_trace!("Plugin is not in realtime mode, bailing!");
            return false;
        }

        //Setup OSC client
        //TODO: cleanup, better error handling
        let socket = UdpSocket::bind("0.0.0.0:0").expect("Could not bind to address");
        let ip_port = format!(
            "{}:{}",
            *self.params.osc_server_address.read(),
            *self.params.osc_server_port.read()
        );
        nih_trace!("Connecting: {}", ip_port);
        socket.connect(ip_port).expect("Connection failed");
        nih_trace!("Connected!");

        let osc_channel = OscChannel::default();
        self.sender = Some(osc_channel.sender);
        let _client_thread = thread::spawn(move || osc_client_worker(socket, osc_channel.receiver));

        true
    }

    fn deactivate(&mut self) {
        nih_trace!("Deactivate Called");
        if let Some(sender) = &self.sender {
            sender.send(OscChannelMessageType::Exit).unwrap();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        //TODO: better error handling
        //TODO: support other midi event types
        let osc_address_base = self.params.osc_address_base.read();
        if let Some(sender) = &self.sender {
            //Process Dirty Params
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p1_dirty,
                &self.params.param1,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p2_dirty,
                &self.params.param2,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p3_dirty,
                &self.params.param3,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p4_dirty,
                &self.params.param4,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p5_dirty,
                &self.params.param5,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p6_dirty,
                &self.params.param6,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p7_dirty,
                &self.params.param7,
            );
            send_dirty_param(
                sender,
                &osc_address_base,
                &self.p8_dirty,
                &self.params.param8,
            );

            //Process Note Events
            if self.params.flag_send_midi.value {
                while let Some(event) = context.next_event() {
                    nih_trace!("NoteEvent: {:?}", event);
                    match event {
                        NoteEvent::NoteOn {
                            timing: _,
                            channel,
                            note,
                            velocity,
                            voice_id: _
                        } => sender
                            .send(OscChannelMessageType::NoteOn(OscNoteType {
                                address_base: osc_address_base.to_string(),
                                channel,
                                note,
                                velocity,
                            }))
                            .unwrap(),
                        NoteEvent::NoteOff {
                            timing: _,
                            channel,
                            note,
                            velocity,
                            voice_id: _    
                        } => sender
                            .send(OscChannelMessageType::NoteOff(OscNoteType {
                                address_base: osc_address_base.to_string(),
                                channel,
                                note,
                                velocity,
                            }))
                            .unwrap(),
                        _ => {}
                    }
                }
            }

            //Process Audio Events
            if self.params.flag_send_audio.value {
                //TODO: deal with a create mono signal or send out multiple channels?
                let mut resampler = FftFixedIn::<f32>::new(
                    44000,
                    100,
                    buffer.len(),
                    128, //let it calculate
                    2,
                ).unwrap();
                
                let downsampled = resampler.process(&buffer.as_slice(), None).unwrap();
                //for channel in downsampled {
                    for &sample in &downsampled[0] { //only grab the first channel?
                        if sample == 0.0 {
                            continue;
                        }
                        sender
                        .send(OscChannelMessageType::Audio(OscAudioType {
                            address_base: osc_address_base.to_string(),
                            value: sample,
                        })).unwrap();
                    }
                //}
            }
        }
        ProcessStatus::Normal
    }
    
    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        nih_trace!("BusConfig: {:?}", config);
        config.num_input_channels == Self::DEFAULT_NUM_INPUTS
            && config.num_output_channels == Self::DEFAULT_NUM_OUTPUTS
    }
}

fn send_dirty_param(
    sender: &Sender<OscChannelMessageType>,
    osc_address_base: &RwLockReadGuard<String>,
    param_dirty: &Arc<AtomicBool>,
    param: &FloatParam,
) {
    if param_dirty
        .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        nih_trace!("Param Dirty: {} {}", param.name(), param.value);
        sender
            .send(OscChannelMessageType::Param(OscParamType {
                address_base: osc_address_base.to_string(),
                name: param.name().to_string(),
                value: param.value,
            }))
            .unwrap();
    }
}

enum OscMidiMessageType {
    NoteOff = 0x80,
    NoteOn = 0x90,
    // PolyPressure = 0xA0,
    // ControlChange = 0xB0,
    // ProgramChange = 0xC0,
    // ChannelPressure = 0xD0,
    // PitchBend = 0xE0,
    // SystemExclusive = 0xF0,
}

// /<osc_address_base>/param/<param_name>
// /<osc_address_base>/midi
// /<osc_address_base>/audio

fn osc_client_worker(socket: UdpSocket, recv: Receiver<OscChannelMessageType>) -> () {
    //TODO: remove expects
    //TODO: should OSC MIDI port always be 0?
    //TODO: handle empty osc_address_base
    while let Some(channel_message) = recv.recv().ok() {
        let osc_message = match channel_message {
            OscChannelMessageType::Exit => break,
            OscChannelMessageType::Param(message) => OscMessage {
                addr: format!("/{}/param/{}", message.address_base, message.name),
                args: vec![OscType::Float(message.value)],
            },
            OscChannelMessageType::NoteOn(message) => OscMessage {
                addr: format!("/{}/midi", message.address_base),
                args: vec![OscType::Midi(OscMidiMessage {
                    port: 0,
                    status: (OscMidiMessageType::NoteOn as u8 | message.channel),
                    data1: message.note,
                    data2: (message.velocity * 127.0) as u8,
                })],
            },
            OscChannelMessageType::NoteOff(message) => OscMessage {
                addr: format!("/{}/midi", message.address_base),
                args: vec![OscType::Midi(OscMidiMessage {
                    port: 0,
                    status: (OscMidiMessageType::NoteOff as u8 | message.channel),
                    data1: message.note,
                    data2: (message.velocity * 127.0) as u8,
                })],
            },
            OscChannelMessageType::Audio(message) => OscMessage {
                addr: format!("/{}/audio", message.address_base),
                args: vec![OscType::Float(message.value)],
            },
        };
        let packet = OscPacket::Message(osc_message);
        let buf = rosc::encoder::encode(&packet).expect("Bad OSC Data");
        let len = socket.send(&buf[..]).expect("Failed to send data");
        if len != buf.len() {
            nih_trace!("UDP packet not fully sent");
        }
        nih_trace!("Sent {:?} packet", packet);
    }
}

impl ClapPlugin for DawOut {
    const CLAP_ID: &'static str = "com.gamingrobot.daw-out";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Outputs MIDI/OSC information from the DAW");
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];

    const CLAP_MANUAL_URL: Option<&'static str> = None;

    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = None;
}

impl Vst3Plugin for DawOut {
    const VST3_CLASS_ID: [u8; 16] = *b"grbt-daw-outputs";
    const VST3_CATEGORIES: &'static str = "Instrument|Tools";
}

nih_export_clap!(DawOut);
nih_export_vst3!(DawOut);
