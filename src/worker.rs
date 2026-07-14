use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;

use crate::monitor::{self, MonitorInfo, SwitchOutcome};

enum WorkerCommand {
    Scan,
    Switch { monitor_id: String, input: u32 },
    Shutdown,
}

#[derive(Debug)]
pub enum WorkerEvent {
    ScanFinished(Result<Vec<MonitorInfo>, String>),
    SwitchFinished {
        monitor_id: String,
        input: u32,
        result: Result<SwitchOutcome, String>,
    },
}

pub struct MonitorWorker {
    command_sender: Sender<WorkerCommand>,
    event_receiver: Receiver<WorkerEvent>,
}

impl MonitorWorker {
    pub fn start(context: egui::Context) -> Self {
        let (command_sender, command_receiver) = unbounded();
        let (event_sender, event_receiver) = unbounded();

        thread::Builder::new()
            .name("monitor-ddc-worker".into())
            .spawn(move || {
                while let Ok(command) = command_receiver.recv() {
                    let event = match command {
                        WorkerCommand::Scan => WorkerEvent::ScanFinished(
                            monitor::scan_monitors().map_err(|error| error.to_string()),
                        ),
                        WorkerCommand::Switch { monitor_id, input } => {
                            let result = monitor::set_input(&monitor_id, input)
                                .map_err(|error| error.to_string());
                            WorkerEvent::SwitchFinished {
                                monitor_id,
                                input,
                                result,
                            }
                        }
                        WorkerCommand::Shutdown => break,
                    };

                    if event_sender.send(event).is_err() {
                        break;
                    }
                    context.request_repaint();
                }
            })
            .expect("failed to start DDC/CI worker thread");

        Self {
            command_sender,
            event_receiver,
        }
    }

    pub fn scan(&self) {
        let _ = self.command_sender.send(WorkerCommand::Scan);
    }

    pub fn switch_input(&self, monitor_id: impl Into<String>, input: u32) {
        let _ = self.command_sender.send(WorkerCommand::Switch {
            monitor_id: monitor_id.into(),
            input,
        });
    }

    pub fn events(&self) -> impl Iterator<Item = WorkerEvent> + '_ {
        self.event_receiver.try_iter()
    }
}

impl Drop for MonitorWorker {
    fn drop(&mut self) {
        let _ = self.command_sender.send(WorkerCommand::Shutdown);
    }
}
