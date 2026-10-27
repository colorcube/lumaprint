use std::io::{BufRead, BufReader};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calloop::{LoopHandle, PostAction, RegistrationToken};
use rustix::pipe::{pipe_with, PipeFlags};
use sctk::compositor::SurfaceData;
use sctk::data_device_manager::data_source::DataSourceData;
use sctk::data_device_manager::ReadPipe;
use sctk::globals::GlobalData;
use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::reexports::client::protocol::wl_data_device::{Event as WlDataDeviceEvent, WlDataDevice};
use sctk::reexports::client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use sctk::reexports::client::protocol::wl_data_offer::{Event as WlDataOfferEvent, WlDataOffer};
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::{
    delegate_dispatch, event_created_child, Connection, Dispatch, Proxy, QueueHandle,
};
use tracing::warn;

use crate::dpi::{LogicalPosition, PhysicalPosition};
use crate::event::WindowEvent;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::{self, DeviceId, WindowId};

const FILE_TRANSFER_MIME_TYPE: &str = "text/uri-list";

pub struct DataDeviceManager {
    manager: WlDataDeviceManager,
}

impl DataDeviceManager {
    pub fn new(
        globals: &GlobalList,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Result<Self, BindError> {
        let manager = globals.bind(queue_handle, 1..=3, GlobalData)?;
        Ok(Self { manager })
    }

    pub fn init_seat_data_device(&self, seat: &WlSeat, queue_handle: &QueueHandle<WinitState>) {
        let data = DataDeviceData { seat: seat.clone() };
        self.manager.get_data_device(seat, queue_handle, data);
    }
}

impl Dispatch<WlDataDeviceManager, GlobalData, WinitState> for DataDeviceState {
    fn event(
        _state: &mut WinitState,
        _proxy: &WlDataDeviceManager,
        _event: <WlDataDeviceManager as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

pub struct DataDeviceData {
    seat: WlSeat,
}

impl Dispatch<WlDataDevice, DataDeviceData, WinitState> for DataDeviceState {
    event_created_child!(WinitState, WlDataDevice, [
        0 => (WlDataOffer, Default::default())
    ]);

    fn event(
        state: &mut WinitState,
        wl_data_device: &WlDataDevice,
        event: <WlDataDevice as Proxy>::Event,
        data: &DataDeviceData,
        conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        let seat_state = match state.seats.get_mut(&data.seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received data device event {event:?} without seat");
                return;
            },
        };

        let data_device_state = match seat_state.data_device_state.as_mut() {
            Some(data_device_state) => data_device_state,
            None => {
                warn!("Received data device event {event:?} without data device state");
                return;
            },
        };

        let device_id = crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId));

        match event {
            WlDataDeviceEvent::Enter { serial, surface, x, y, id } => {
                let Some(wl_data_offer) = id else {
                    warn!("Received data device enter event without id");
                    return;
                };

                let Some(data_offer) = wl_data_offer.data::<DataOfferData>() else {
                    warn!("Received data device enter event without data offer");
                    return;
                };

                let Some(mime_type) = (match data_offer.mime_types.lock() {
                    Ok(mime_types) => mime_types
                        .iter()
                        .find(|&mime| {
                            mime == FILE_TRANSFER_MIME_TYPE
                                || mime.starts_with("text/uri-list;")
                        })
                        .cloned(),
                    Err(e) => {
                        warn!("Failed to lock MIME type vector: {e}");
                        None
                    },
                }) else {
                    if wl_data_offer.version() >= 3 {
                        wl_data_offer.set_actions(DndAction::None, DndAction::None);
                    }
                    wl_data_offer.accept(serial, None);
                    let _ = conn.flush();
                    return;
                };

                let parent_surface = surface
                    .data::<SurfaceData>()
                    .and_then(|d| d.parent_surface())
                    .unwrap_or(&surface);
                let window_id = wayland::make_wid(parent_surface);

                let scale_factor = state
                    .windows
                    .get_mut()
                    .get(&window_id)
                    .and_then(|window| window.lock().ok().map(|w| w.scale_factor()))
                    .unwrap_or(1.0);
                let position: PhysicalPosition<f64> =
                    LogicalPosition::new(x, y).to_physical(scale_factor);
                state.events_sink.push_window_event(
                    WindowEvent::CursorMoved { device_id, position },
                    window_id,
                );

                // Start unaccepted (forbidden cursor) until the URI list pipe is read and verified
                if wl_data_offer.version() >= 3 {
                    wl_data_offer.set_actions(DndAction::None, DndAction::None);
                }
                wl_data_offer.accept(serial, None);

                let (read_fd, write_fd) = match pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK)
                {
                    Ok((read_fd, write_fd)) => (read_fd, write_fd),
                    Err(e) => {
                        warn!("Failed to create pipe to read data offer from: {e}");
                        return;
                    },
                };

                wl_data_offer.receive(mime_type.clone(), write_fd.as_fd());
                drop(write_fd);
                let _ = conn.flush();

                let read_pipe = ReadPipe::from(read_fd);

                if let Some(token) = data_device_state.read_token.take() {
                    data_device_state.loop_handle.remove(token);
                }

                data_device_state.hovered_window = Some(window_id);
                data_device_state.serial = Some(serial);
                data_device_state.mime_type = Some(mime_type);
                data_device_state.offer = Some(wl_data_offer);
                data_device_state.read_buffer.clear();
                data_device_state.read_completed = false;
                data_device_state.offer_accepted = false;
                data_device_state.drop_pending = false;

                let wl_data_device = wl_data_device.clone();
                let conn_clone = conn.clone();
                data_device_state.read_token = data_device_state
                    .loop_handle
                    .insert_source(read_pipe, move |_, f, state| {
                        state.dispatched_events = true;
                        let Some(data) = wl_data_device.data::<DataDeviceData>() else {
                            return PostAction::Remove;
                        };

                        let Some(seat_state) = state.seats.get_mut(&data.seat.id()) else {
                            return PostAction::Remove;
                        };

                        let Some(data_device_state) = seat_state.data_device_state.as_mut() else {
                            return PostAction::Remove;
                        };

                        let f: &mut std::fs::File = unsafe { f.get_mut() };
                        let mut reader = BufReader::new(f);

                        let consumed = match reader.fill_buf() {
                            Ok(buf) => {
                                if buf.is_empty() {
                                    data_device_state.read_completed = true;
                                    if let Some(hovered) = data_device_state.hovered_window {
                                        let drop_pending = data_device_state.drop_pending;
                                        let accepted_paths = data_device_state.accepted_file_paths();
                                        let is_accepted = !accepted_paths.is_empty();
                                        data_device_state.offer_accepted = is_accepted;

                                        if let (Some(offer), Some(serial), Some(mime)) = (
                                            &data_device_state.offer,
                                            data_device_state.serial,
                                            &data_device_state.mime_type,
                                        ) {
                                            if is_accepted {
                                                if offer.version() >= 3 {
                                                    offer.set_actions(
                                                        DndAction::Copy | DndAction::Move,
                                                        DndAction::Copy,
                                                    );
                                                }
                                                offer.accept(serial, Some(mime.clone()));
                                            } else {
                                                if offer.version() >= 3 {
                                                    offer.set_actions(
                                                        DndAction::None,
                                                        DndAction::None,
                                                    );
                                                }
                                                offer.accept(serial, None);
                                            }
                                            let _ = conn_clone.flush();
                                        }

                                        for path in accepted_paths {
                                            if drop_pending {
                                                state.events_sink.push_window_event(
                                                    WindowEvent::DroppedFile(path),
                                                    hovered,
                                                );
                                            } else {
                                                state.events_sink.push_window_event(
                                                    WindowEvent::HoveredFile(path),
                                                    hovered,
                                                );
                                            }
                                        }
                                        if drop_pending {
                                            data_device_state.finish_offer(true);
                                        }
                                    }
                                    return PostAction::Remove;
                                } else {
                                    data_device_state.read_buffer.extend_from_slice(buf);
                                }
                                buf.len()
                            },
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::Interrupted
                                        | std::io::ErrorKind::WouldBlock
                                ) =>
                            {
                                return PostAction::Continue;
                            },
                            Err(_) => {
                                return PostAction::Remove;
                            },
                        };
                        reader.consume(consumed);

                        PostAction::Continue
                    })
                    .ok();
            },
            WlDataDeviceEvent::Motion { time: _, x, y } => {
                if data_device_state.read_completed {
                    if let (Some(offer), Some(serial), Some(mime)) = (
                        &data_device_state.offer,
                        data_device_state.serial,
                        &data_device_state.mime_type,
                    ) {
                        if data_device_state.offer_accepted {
                            if offer.version() >= 3 {
                                offer.set_actions(
                                    DndAction::Copy | DndAction::Move,
                                    DndAction::Copy,
                                );
                            }
                            offer.accept(serial, Some(mime.clone()));
                        } else {
                            if offer.version() >= 3 {
                                offer.set_actions(DndAction::None, DndAction::None);
                            }
                            offer.accept(serial, None);
                        }
                        let _ = conn.flush();
                    }
                }

                if let Some(window_id) = data_device_state.hovered_window {
                    let scale_factor = state
                        .windows
                        .get_mut()
                        .get(&window_id)
                        .and_then(|window| window.lock().ok().map(|w| w.scale_factor()))
                        .unwrap_or(1.0);
                    let position: PhysicalPosition<f64> =
                        LogicalPosition::new(x, y).to_physical(scale_factor);
                    state.events_sink.push_window_event(
                        WindowEvent::CursorMoved { device_id, position },
                        window_id,
                    );
                }
            },
            WlDataDeviceEvent::Drop => {
                if data_device_state.read_completed {
                    if let Some(hovered_window) = data_device_state.hovered_window {
                        for path in data_device_state.accepted_file_paths() {
                            state.events_sink.push_window_event(
                                WindowEvent::DroppedFile(path),
                                hovered_window,
                            );
                        }
                        data_device_state.finish_offer(true);
                    }
                } else {
                    data_device_state.drop_pending = true;
                }
            },
            WlDataDeviceEvent::Leave => {
                if !data_device_state.drop_pending {
                    if let Some(hovered_window) = data_device_state.hovered_window {
                        if data_device_state.offer_accepted {
                            state.events_sink.push_window_event(
                                WindowEvent::HoveredFileCancelled,
                                hovered_window,
                            );
                        }
                        data_device_state.finish_offer(false);
                    }
                }
            },
            WlDataDeviceEvent::DataOffer { id: _ } => {},
            WlDataDeviceEvent::Selection { id: _ } => {},
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
pub struct DataOfferData {
    pub mime_types: Arc<Mutex<Vec<String>>>,
}

impl Dispatch<WlDataOffer, DataOfferData, WinitState> for DataDeviceState {
    fn event(
        _state: &mut WinitState,
        _wl_data_offer: &WlDataOffer,
        event: <WlDataOffer as Proxy>::Event,
        data: &DataOfferData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        match event {
            WlDataOfferEvent::SourceActions { source_actions: _ } => {},
            WlDataOfferEvent::Action { dnd_action: _ } => {},
            WlDataOfferEvent::Offer { mime_type } => match data.mime_types.lock() {
                Ok(mut mime_types) => {
                    mime_types.push(mime_type);
                },
                Err(e) => {
                    warn!("Failed to lock data offer mime types: {e}");
                },
            },
            _ => unreachable!(),
        }
    }
}

impl Dispatch<WlDataSource, DataSourceData, WinitState> for DataDeviceState {
    fn event(
        _state: &mut WinitState,
        _proxy: &WlDataSource,
        _event: <WlDataSource as Proxy>::Event,
        _data: &DataSourceData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

#[derive(Debug)]
pub struct DataDeviceState {
    pub loop_handle: LoopHandle<'static, WinitState>,
    pub hovered_window: Option<WindowId>,
    pub serial: Option<u32>,
    pub mime_type: Option<String>,
    pub offer: Option<WlDataOffer>,
    pub read_token: Option<RegistrationToken>,
    pub read_buffer: Vec<u8>,
    pub read_completed: bool,
    pub offer_accepted: bool,
    pub drop_pending: bool,
}

impl DataDeviceState {
    pub fn new(loop_handle: LoopHandle<'static, WinitState>) -> Self {
        Self {
            loop_handle,
            hovered_window: None,
            serial: None,
            mime_type: None,
            offer: None,
            read_token: None,
            read_buffer: Vec::new(),
            read_completed: false,
            offer_accepted: false,
            drop_pending: false,
        }
    }

    fn accepted_file_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(uri_list) = String::from_utf8(self.read_buffer.clone()) {
            for line in uri_list.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                if let Some(raw_path) = line
                    .strip_prefix("file://localhost")
                    .or_else(|| line.strip_prefix("file://"))
                {
                    let p = PathBuf::from(percent_decode_str(raw_path));
                    if is_accepted_drop_path(&p) {
                        paths.push(p);
                    }
                } else {
                    warn!("Non-comment line in URI list missing prefix: '{line}'");
                }
            }
        } else {
            warn!("Failed to parse URI list from hovered file");
        }
        paths
    }

    fn finish_offer(&mut self, is_drop: bool) {
        let was_accepted = self.offer_accepted;
        self.hovered_window = None;
        self.serial = None;
        self.mime_type = None;
        self.read_completed = false;
        self.offer_accepted = false;
        self.drop_pending = false;
        if let Some(offer) = self.offer.take() {
            if is_drop && was_accepted && offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
        }
        if let Some(token) = self.read_token.take() {
            self.loop_handle.remove(token);
        }
    }
}

fn is_accepted_drop_path(path: &std::path::Path) -> bool {
    if path.is_dir() {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "tif" | "tiff" | "png" | "jpg" | "jpeg"
            )
        })
        .unwrap_or(false)
}

fn percent_decode_str(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(output).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

delegate_dispatch!(WinitState: [WlDataDeviceManager: GlobalData] => DataDeviceState);
delegate_dispatch!(WinitState: [WlDataDevice: DataDeviceData] => DataDeviceState);
delegate_dispatch!(WinitState: [WlDataOffer: DataOfferData] => DataDeviceState);
delegate_dispatch!(WinitState: [WlDataSource: DataSourceData] => DataDeviceState);
