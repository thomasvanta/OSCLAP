use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use nih_plug::debug::*;
use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use parking_lot::RwLock;
use rosc::{OscMessage, OscPacket, OscType};
use rubato::{FftFixedOut, Resampler};
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::ops::Index;

mod editor;
mod subviews;

pub struct OsClap {
    params: Arc<OsClapParams>,
    osc_thread: Option<JoinHandle<()>>,
    sender: Arc<Sender<OscChannelMessageType>>,
    receiver: Option<Receiver<OscChannelMessageType>>,
    editor_state: Arc<ViziaState>,
    input_sample_rate: f32,
    resampler: Option<FftFixedOut<f32>>,
    resampler_buffer: Option<Vec<Vec<f32>>>,
    p1_dirty: Arc<AtomicBool>,
    p2_dirty: Arc<AtomicBool>,
    p3_dirty: Arc<AtomicBool>,
    p4_dirty: Arc<AtomicBool>,
    p5_dirty: Arc<AtomicBool>,
    p6_dirty: Arc<AtomicBool>,
    p7_dirty: Arc<AtomicBool>,
    p8_dirty: Arc<AtomicBool>,
}

impl Default for OsClap {
    fn default() -> Self {
        let p1_dirty = Arc::new(AtomicBool::new(false));
        let p2_dirty = Arc::new(AtomicBool::new(false));
        let p3_dirty = Arc::new(AtomicBool::new(false));
        let p4_dirty = Arc::new(AtomicBool::new(false));
        let p5_dirty = Arc::new(AtomicBool::new(false));
        let p6_dirty = Arc::new(AtomicBool::new(false));
        let p7_dirty = Arc::new(AtomicBool::new(false));
        let p8_dirty = Arc::new(AtomicBool::new(false));

        let channel = OscChannel::default();
        Self {
            params: Arc::new(OsClapParams::new(
                p1_dirty.clone(),
                p2_dirty.clone(),
                p3_dirty.clone(),
                p4_dirty.clone(),
                p5_dirty.clone(),
                p6_dirty.clone(),
                p7_dirty.clone(),
                p8_dirty.clone(),
            )),
            osc_thread: None,
            sender: Arc::new(channel.sender),
            receiver: Some(channel.receiver),
            input_sample_rate: 1.0,
            resampler: None,
            resampler_buffer: None,
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

impl Drop for OsClap {
    fn drop(&mut self) {
        self.kill_background_thread();
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
    name: String,
    value: f32,
}

struct OscNoteType {
    channel: u8,
    note: u8,
    velocity: f32,
}

struct OscAudioType {
    value: f32,
}

struct OscConnectionType {
    ip: String,
    port: u16,
}

struct OscAddressBaseType {
    address: String,
}



enum OscChannelMessageType {
    Exit,
    ConnectionChange(OscConnectionType),
    AddressBaseChange(OscAddressBaseType),
    Param(OscParamType),
    NoteOn(OscNoteType),
    NoteOff(OscNoteType),
    Audio(OscAudioType),
}

#[derive(Params)]
pub struct OsClapParams {
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
    #[id = "osc_sample_rate"]
    osc_sample_rate: IntParam,

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

impl Index<usize> for OsClapParams {
    type Output = FloatParam;

    fn index(&self, index: usize) -> &Self::Output {
        match index {
            0 => &self.param1,
            1 => &self.param2,
            2 => &self.param3,
            3 => &self.param4,
            4 => &self.param5,
            5 => &self.param6,
            6 => &self.param7,
            7 => &self.param8,
            n => panic!("Invalid Parameter index: {}", n)
        }
    }
}

impl OsClapParams {
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
            osc_server_address: RwLock::new("255.255.255.255".to_string()),
            osc_server_port: RwLock::new(12345),
            osc_address_base: RwLock::new("osclap".to_string()),
            flag_send_midi: BoolParam::new("flag_send_midi", true)
                .hide()
                .non_automatable(),
            flag_send_audio: BoolParam::new("flag_send_audio", false)
                .hide()
                .non_automatable(),
            //TODO: handle value change updating resampler ratio
            osc_sample_rate: IntParam::new(
                "osc_sample_rate",
                100,
                IntRange::Linear { min: 0, max: 1000 },
            )
            .hide()
            .non_automatable(),
            param1: FloatParam::new("param1", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p1_dirty.store(true, Ordering::Release))),
            param2: FloatParam::new("param2", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p2_dirty.store(true, Ordering::Release))),
            param3: FloatParam::new("param3", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p3_dirty.store(true, Ordering::Release))),
            param4: FloatParam::new("param4", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p4_dirty.store(true, Ordering::Release))),
            param5: FloatParam::new("param5", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p5_dirty.store(true, Ordering::Release))),
            param6: FloatParam::new("param6", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p6_dirty.store(true, Ordering::Release))),
            param7: FloatParam::new("param7", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p7_dirty.store(true, Ordering::Release))),
            param8: FloatParam::new("param8", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_step_size(0.001)
                .with_callback(Arc::new(move |_x| p8_dirty.store(true, Ordering::Release))),
        }
    }
}

impl Plugin for OsClap {
    const NAME: &'static str = "OSCLAP";
    const VENDOR: &'static str = "VanTa";
    const URL: &'static str = "vanta.xyz";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    const HARD_REALTIME_ONLY: bool = true;

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),

        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        nih_trace!("Params Called");
        self.params.clone() as Arc<dyn Params>
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        nih_trace!("Editor Called");
        editor::create(
            self.params.clone(),
            self.sender.clone(),
            self.editor_state.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        nih_trace!("Initialize Called");

        if buffer_config.process_mode != ProcessMode::Realtime {
            nih_log!("Plugin is not in realtime mode, bailing!");
            return false;
        }

        //Setup resampler
        self.input_sample_rate = buffer_config.sample_rate;
        self.resampler = match FftFixedOut::<f32>::new(
            self.input_sample_rate as usize / 100, //TODO: is this right?
            self.params.osc_sample_rate.value() as usize,
            100,
            2,
            2,
        ) {
            Ok(sampler) => Some(sampler),
            Err(e) => {
                nih_error!(
                    "Failed to create resampler, audio processing will be disabled {:?}",
                    e
                );
                None
            }
        };

        if let Some(resampler) = &self.resampler {
            self.resampler_buffer = Some(resampler.output_buffer_allocate(true));
        }

        //Setup OSC background thread
        //Dont remake the background thread if its already running
        if self.osc_thread.is_none() {
            let socket = match UdpSocket::bind("0.0.0.0:0") {
                Ok(socket) => socket,
                Err(e) => {
                    nih_error!("Failed to bind socket {:?}", e);
                    return false;
                }
            };
            let ip_port = format!(
                "{}:{}",
                *self.params.osc_server_address.read(),
                *self.params.osc_server_port.read()
            );
            nih_trace!("Connecting: {}", ip_port);
            
            socket.set_broadcast(true);

            let connect_result = socket.connect(&ip_port);
            if connect_result.is_err() {
                nih_error!(
                    "Failed to connect socket to {} {:?}",
                    ip_port,
                    connect_result.unwrap_err()
                );
                return false;
            }

            nih_trace!("Connected!");
            nih_trace!("Connected to: {}", ip_port);

            let address_base = self.params.osc_address_base.read().to_string();
            nih_trace!("OSC Address Base: {}", address_base);

            if let Some(receiver) = std::mem::replace(&mut self.receiver, None) {
                let client_thread =
                    thread::spawn(move || osc_client_worker(socket, address_base, receiver));

                self.osc_thread = Some(client_thread);
            } else {
                nih_error!("Failed get thread channel receiver");
                return false;
            }
        } else {
            //Threads already alive just update params
            let connection_send_result =
                self.sender
                    .send(OscChannelMessageType::ConnectionChange(OscConnectionType {
                        ip: self.params.osc_server_address.read().to_string(),
                        port: *self.params.osc_server_port.read(),
                    }));
            if connection_send_result.is_err() {
                nih_error!(
                    "Failed to send ConnectionChange update {:?}",
                    connection_send_result.unwrap_err()
                );
            }
            let address_base = self.params.osc_address_base.read().to_string();
            nih_trace!("OSC Address Base: {}", address_base);
            let address_send_result = self.sender.send(OscChannelMessageType::AddressBaseChange(
                OscAddressBaseType {
                    address: address_base,
                },
            ));
            if address_send_result.is_err() {
                nih_error!(
                    "Failed to send AddressBaseChange update {:?}",
                    address_send_result.unwrap_err()
                );
            }
        }
        true
    }

    fn deactivate(&mut self) {
        nih_trace!("Deactivate Called");
        self.kill_background_thread();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        //Process Dirty Params
        let param_result = self.process_params();
        if param_result.is_err() {
            nih_error!("Failed to send params {:?}", param_result.unwrap_err());
        }
        //Process Note Events
        if self.params.flag_send_midi.value() {
            while let Some(event) = context.next_event() {
                nih_trace!("NoteEvent: {:?}", event);
                let message_result = self.process_event(&event);
                if message_result.is_err() {
                    nih_error!(
                        "Failed to process NoteEvent {:?}",
                        message_result.unwrap_err()
                    );
                }
            }
        }
        //Process Audio Events
        if self.params.flag_send_audio.value() {
            let audio_result = self.process_audio_buffer(buffer);
            if audio_result.is_err() {
                nih_error!("Failed to process Audio {:?}", audio_result.unwrap_err());
            }
        }
        ProcessStatus::Normal
    }
}

impl OsClap {
    fn process_params(&self) -> Result<()> {
        self.send_dirty_param(&self.p1_dirty, &self.params.param1)?;
        self.send_dirty_param(&self.p2_dirty, &self.params.param2)?;
        self.send_dirty_param(&self.p3_dirty, &self.params.param3)?;
        self.send_dirty_param(&self.p4_dirty, &self.params.param4)?;
        self.send_dirty_param(&self.p5_dirty, &self.params.param5)?;
        self.send_dirty_param(&self.p6_dirty, &self.params.param6)?;
        self.send_dirty_param(&self.p7_dirty, &self.params.param7)?;
        self.send_dirty_param(&self.p8_dirty, &self.params.param8)?;
        Ok(())
    }

    fn send_dirty_param(&self, param_dirty: &Arc<AtomicBool>, param: &FloatParam) -> Result<()> {
        if param_dirty
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            nih_trace!("Param Dirty: {} {}", param.name(), param.value());
            self.sender
                .send(OscChannelMessageType::Param(OscParamType {
                    name: param.name().to_string(), //TODO: allocation
                    value: param.value(),
                }))?;
        }
        Ok(())
    }

    fn process_event(&self, event: &NoteEvent<()>) -> Result<()> {
        match *event {
            NoteEvent::NoteOn {
                timing: _,
                channel,
                note,
                velocity,
                voice_id: _,
            } => self
                .sender
                .send(OscChannelMessageType::NoteOn(OscNoteType {
                    channel,
                    note,
                    velocity,
                }))?,
            NoteEvent::NoteOff {
                timing: _,
                channel,
                note,
                velocity,
                voice_id: _,
            } => self
                .sender
                .send(OscChannelMessageType::NoteOff(OscNoteType {
                    channel,
                    note,
                    velocity,
                }))?,
            _ => {}
        };
        Ok(())
    }

    fn process_audio_buffer(&mut self, buffer: &mut Buffer) -> Result<()> {
        if let Some(resampler) = &mut self.resampler {
            if let Some(resampler_buffer) = &mut self.resampler_buffer {
                //TODO: deal with a create mono signal or send out multiple channels?
                resampler.process_into_buffer(&buffer.as_slice(), resampler_buffer, None)?;
                //TODO: we only use the first channel
                for &sample in &resampler_buffer[0] {
                    if sample == 0.0 {
                        continue;
                    }
                    let send_result = self
                        .sender
                        .send(OscChannelMessageType::Audio(OscAudioType { value: sample }));
                    if send_result.is_err() {
                        nih_error!("Failed to send processed audio {:?}", send_result.unwrap_err());
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn kill_background_thread(&mut self) {
        let exit_result = self.sender.send(OscChannelMessageType::Exit);
        if exit_result.is_err() {
            nih_error!(
                "Failed to send shutdown to background thread {:?}",
                exit_result.unwrap_err()
            );
        }
        self.osc_thread = None;
    }
}

// /<osc_address_base>/param/<param_name>
// /<osc_address_base>/note_on <channel> <note> <velocity>
// /<osc_address_base>/note_off <channel> <note> <velocity>
// /<osc_address_base>/audio

fn osc_client_worker(
    socket: UdpSocket,
    param_address_base: String,
    recv: Receiver<OscChannelMessageType>,
) -> () {
    nih_trace!("Background thread spawned!");
    nih_trace!("Background thread OSC Address Base: {}", param_address_base);
    let mut address_base = format_osc_address_base(&param_address_base);
    let mut connected = true; //We assume the socket we get is good
    while let Some(channel_message) = recv.recv().ok() {
        let osc_message = match channel_message {
            OscChannelMessageType::Exit => break,
            OscChannelMessageType::ConnectionChange(message) => {
                let ip_port = format!("{}:{}", message.ip, message.port);
                nih_trace!("Connection Change: {}", ip_port);
                let socket_result = socket.connect(&ip_port);
                match socket_result {
                    Ok(_) => connected = true,
                    Err(e) => {
                        connected = false;
                        nih_error!("Failed to connect to {} {:?}", ip_port, e);
                    }
                }
                continue;
            }
            OscChannelMessageType::AddressBaseChange(message) => {
                address_base = format_osc_address_base(&message.address);
                nih_trace!("AddressBase Change: {}", address_base);
                continue;
            }
            OscChannelMessageType::Param(message) => OscMessage {
                addr: format!("{}/param/{}", address_base, message.name),
                args: vec![OscType::Float(message.value)],
            },
            OscChannelMessageType::NoteOn(message) => OscMessage {
                addr: format!("{}/note_on", address_base),
                args: vec![
                    OscType::Int(message.channel as i32),
                    OscType::Int(message.note as i32),
                    OscType::Float(message.velocity),
                ],
            },
            OscChannelMessageType::NoteOff(message) => OscMessage {
                addr: format!("{}/note_off", address_base),
                args: vec![
                    OscType::Int(message.channel as i32),
                    OscType::Int(message.note as i32),
                    OscType::Float(message.velocity),
                ],
            },
            OscChannelMessageType::Audio(message) => OscMessage {
                addr: format!("{}/audio", address_base),
                args: vec![OscType::Float(message.value)],
            },
        };
        if connected {
            let packet = OscPacket::Message(osc_message);
            let buf = match rosc::encoder::encode(&packet) {
                Ok(buf) => buf,
                Err(e) => {
                    nih_error!("Failed to encode osc message {:?}", e);
                    continue;
                }
            };
            let len = match socket.send(&buf[..]) {
                Ok(buf) => buf,
                Err(e) => {
                    nih_error!("Failed to send osc message {:?}", e);
                    continue;
                }
            };
            if len != buf.len() {
                nih_trace!("UDP packet not fully sent");
            }
            nih_trace!("Sent {:?} packet", packet);
        }
    }
}

fn format_osc_address_base(raw_base: &str) -> String {
    if raw_base.is_empty() {
        return "".to_string();
    } else {
        return format!("/{}", raw_base); //Prefix with slash
    }
}

impl ClapPlugin for OsClap {
    const CLAP_ID: &'static str = "xyz.vanta.osclap";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Outputs MIDI/OSC information from the DAW");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::NoteEffect,
        ClapFeature::Utility,
        ClapFeature::Analyzer,
    ];

    const CLAP_MANUAL_URL: Option<&'static str> = None;

    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = None;
}

// impl Vst3Plugin for OsClap {
//     const VST3_CLASS_ID: [u8; 16] = *b"grbt-daw-outputs";
//     const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Tools];
// }

nih_export_clap!(OsClap);
//nih_export_vst3!(OsClap);
