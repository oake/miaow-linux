use async_channel::{Receiver, Sender};
use uuid::Uuid;

use crate::{
    call::{CallCommand, CallEvent, run_call_actor},
    reaction::RemoteReactionSlot,
    reaction_sync::ReactionSync,
    room::{PendingRoom, RoomStore, SavedRoom},
};

#[derive(Clone, Debug)]
pub enum BackendCommand {
    Load {
        selected: Option<Uuid>,
        temporary_remote: Option<String>,
    },
    Select(Uuid),
    Add(PendingRoom),
    DeleteSelected,
    Call(CallCommand),
    SetReactionSlot {
        number: u8,
        assignment: Option<RemoteReactionSlot>,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    RoomsLoaded {
        rooms: Vec<SavedRoom>,
        selected: Option<SavedRoom>,
        remember_selection: bool,
    },
    RoomSelected(SavedRoom),
    RoomAdded(SavedRoom),
    RoomDeleted {
        rooms: Vec<SavedRoom>,
        selected: Option<SavedRoom>,
    },
    PersistenceError(String),
    ReactionSlots {
        identity: String,
        slots: Vec<RemoteReactionSlot>,
    },
    ReactionSyncFinished(u8),
    Call(CallEvent),
    ShutdownComplete,
}

pub async fn run_backend(commands: Receiver<BackendCommand>, events: Sender<BackendEvent>) {
    let (call_tx, call_rx) = async_channel::bounded(32);
    let (call_event_tx, call_event_rx) = async_channel::bounded(8);
    let mut call_handle = Some(tokio::spawn(run_call_actor(call_rx, call_event_tx)));
    let forwarded_events = events.clone();
    tokio::spawn(async move {
        while let Ok(event) = call_event_rx.recv().await {
            if forwarded_events
                .send(BackendEvent::Call(event))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut store: Option<RoomStore> = None;
    let sync = ReactionSync::new();
    let mut reaction_pull: Option<tokio::task::JoinHandle<()>> = None;
    let mut reaction_pushes = std::collections::HashMap::<u8, tokio::task::JoinHandle<()>>::new();
    let mut expiration_check = tokio::time::interval(std::time::Duration::from_secs(30));
    expiration_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let command = tokio::select! {
            command = commands.recv() => match command {
                Ok(command) => command,
                Err(_) => break,
            },
            _ = expiration_check.tick() => {
                if let Some(current) = store.as_mut() {
                    match current.purge_expired().await {
                        Ok(true) => {
                            let selected = current.selected().cloned();
                            let _ = call_tx.send(CallCommand::Select(selected.clone())).await;
                            let _ = events.send(BackendEvent::RoomDeleted {
                                rooms: current.rooms().to_vec(),
                                selected: selected.clone(),
                            }).await;
                            start_reaction_pull(
                                &mut reaction_pull,
                                selected,
                                sync.clone(),
                                events.clone(),
                            );
                        }
                        Ok(false) => {}
                        Err(error) => {
                            let _ = events.send(BackendEvent::PersistenceError(error.to_string())).await;
                        }
                    }
                }
                continue;
            }
        };
        match command {
            BackendCommand::Load {
                selected,
                temporary_remote,
            } => {
                let remember_selection = temporary_remote.is_none();
                match RoomStore::load(selected, temporary_remote.as_deref()).await {
                    Ok(loaded) => {
                        let rooms = loaded.rooms().to_vec();
                        let selected = loaded.selected().cloned();
                        let _ = call_tx.send(CallCommand::Select(selected.clone())).await;
                        store = Some(loaded);
                        let _ = events
                            .send(BackendEvent::RoomsLoaded {
                                rooms,
                                selected: selected.clone(),
                                remember_selection,
                            })
                            .await;
                        start_reaction_pull(
                            &mut reaction_pull,
                            selected,
                            sync.clone(),
                            events.clone(),
                        );
                    }
                    Err(error) => {
                        let _ = events
                            .send(BackendEvent::PersistenceError(error.to_string()))
                            .await;
                    }
                }
            }
            BackendCommand::Select(id) => {
                if let Some(room) = store.as_mut().and_then(|store| store.select(id)) {
                    let _ = call_tx.send(CallCommand::Select(Some(room.clone()))).await;
                    let _ = events.send(BackendEvent::RoomSelected(room.clone())).await;
                    start_reaction_pull(
                        &mut reaction_pull,
                        Some(room),
                        sync.clone(),
                        events.clone(),
                    );
                }
            }
            BackendCommand::Add(pending) => {
                if let Some(existing) = store
                    .as_ref()
                    .and_then(|store| store.exact_token(&pending.token))
                    .cloned()
                {
                    if let Some(store) = store.as_mut() {
                        store.select(existing.id);
                    }
                    let _ = call_tx
                        .send(CallCommand::Select(Some(existing.clone())))
                        .await;
                    let _ = events
                        .send(BackendEvent::RoomSelected(existing.clone()))
                        .await;
                    start_reaction_pull(
                        &mut reaction_pull,
                        Some(existing),
                        sync.clone(),
                        events.clone(),
                    );
                } else if let Some(store) = store.as_mut() {
                    match store.add(pending).await {
                        Ok(room) => {
                            let _ = call_tx.send(CallCommand::Select(Some(room.clone()))).await;
                            let _ = events.send(BackendEvent::RoomAdded(room.clone())).await;
                            start_reaction_pull(
                                &mut reaction_pull,
                                Some(room),
                                sync.clone(),
                                events.clone(),
                            );
                        }
                        Err(error) => {
                            let _ = events
                                .send(BackendEvent::PersistenceError(error.to_string()))
                                .await;
                        }
                    }
                }
            }
            BackendCommand::DeleteSelected => {
                if let Some(store) = store.as_mut() {
                    match store.delete_selected().await {
                        Ok(selected) => {
                            let _ = call_tx.send(CallCommand::Select(selected.clone())).await;
                            let _ = events
                                .send(BackendEvent::RoomDeleted {
                                    rooms: store.rooms().to_vec(),
                                    selected: selected.clone(),
                                })
                                .await;
                            start_reaction_pull(
                                &mut reaction_pull,
                                selected,
                                sync.clone(),
                                events.clone(),
                            );
                        }
                        Err(error) => {
                            let _ = events
                                .send(BackendEvent::PersistenceError(error.to_string()))
                                .await;
                        }
                    }
                }
            }
            BackendCommand::Call(command) => {
                let _ = call_tx.send(command).await;
            }
            BackendCommand::SetReactionSlot { number, assignment } => {
                if let Some(previous) = reaction_pushes.remove(&number) {
                    previous.abort();
                }
                let room = store.as_ref().and_then(RoomStore::selected).cloned();
                let sync = sync.clone();
                let events = events.clone();
                let task = tokio::spawn(async move {
                    if let Some(room) = room {
                        match assignment {
                            Some(slot) => {
                                if let Some(path) = crate::reaction::cached_file_path(&slot.hash)
                                    && let Err(error) = sync.assign(&room, &slot, &path).await
                                {
                                    log::warn!("reaction assignment sync failed: {error:#}");
                                }
                            }
                            None => {
                                if let Err(error) = sync.remove(&room, number).await {
                                    log::warn!("reaction removal sync failed: {error:#}");
                                }
                            }
                        }
                    }
                    let _ = events
                        .send(BackendEvent::ReactionSyncFinished(number))
                        .await;
                });
                reaction_pushes.insert(number, task);
            }
            BackendCommand::Shutdown => {
                if let Some(task) = reaction_pull.take() {
                    task.abort();
                }
                for (_, task) in reaction_pushes.drain() {
                    task.abort();
                }
                let _ = call_tx.send(CallCommand::Shutdown).await;
                if let Some(handle) = call_handle.take() {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
                }
                let _ = events.send(BackendEvent::ShutdownComplete).await;
                break;
            }
        }
    }
}

fn start_reaction_pull(
    task: &mut Option<tokio::task::JoinHandle<()>>,
    room: Option<SavedRoom>,
    sync: ReactionSync,
    events: Sender<BackendEvent>,
) {
    if let Some(previous) = task.take() {
        previous.abort();
    }
    let Some(room) = room else { return };
    *task = Some(tokio::spawn(async move {
        let delays = [1, 2, 4, 8, 15, 30];
        let mut attempt = 0usize;
        loop {
            match sync.pull(&room).await {
                Ok(slots) => {
                    for slot in &slots {
                        if let Err(error) = sync.download_missing(&room, &slot.hash).await {
                            log::warn!("reaction file pull failed for {}: {error:#}", slot.hash);
                        }
                    }
                    let _ = events
                        .send(BackendEvent::ReactionSlots {
                            identity: room.local_identity.clone(),
                            slots,
                        })
                        .await;
                    return;
                }
                Err(error) => {
                    log::warn!("reaction slot pull failed: {error:#}");
                    tokio::time::sleep(std::time::Duration::from_secs(
                        delays[attempt.min(delays.len() - 1)],
                    ))
                    .await;
                    attempt += 1;
                }
            }
        }
    }));
}
