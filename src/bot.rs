use std::fmt::Write;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use azalea::container::ContainerHandle;
use azalea::{
    WalkDirection, prelude::*, protocol::packets::game::ServerboundGamePacket,
    registry::builtin::ItemKind,
};
use tokio::{sync::mpsc::Sender, time};

use crate::{PWD, State, message::Message, server::WsServerMessage};

#[derive(Default, Clone, Component)]
pub enum Instruction {
    Chat(String),
    #[default]
    Jump,
    TabList(Sender<String>),
    Walk(WalkDirection),
    Sneak(bool),
    Slot(u8),
    GetSlot(Sender<u8>),
}

#[derive(Default, Clone, Component, PartialEq, Eq, PartialOrd, Ord)]
pub enum BotLocation {
    #[default]
    Login,
    Lobby,
    BedwarsLobby,
}

pub async fn handle(bot: Client, event: Event, state: State) -> anyhow::Result<()> {
    match event {
        Event::Spawn => {
            tokio::spawn(async move {
                let mut interval = time::interval(Duration::from_secs(5));

                loop {
                    interval.tick().await;
                    let is_in_bedwars_lobby = {
                        let location = state.location.lock().await;
                        location.eq(&BotLocation::BedwarsLobby)
                    };
                    if is_in_bedwars_lobby {
                        tokio::spawn(async move {
                            let mut interval = time::interval(Duration::from_secs(30));
                            loop {
                                interval.tick().await;
                                bot.walk(azalea::WalkDirection::Backward);
                                bot.jump();
                                tokio::time::sleep(Duration::from_secs(4)).await;
                                bot.walk(WalkDirection::Left);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                bot.walk(WalkDirection::Right);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                bot.walk(WalkDirection::Forward);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                bot.walk(WalkDirection::None);

                                let mut inventory: Option<ContainerHandle> = None;
                                _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                                    if let Some(opened_inv) = bot.open_inventory() {
                                        inventory.replace(opened_inv);
                                    }
                                }));

                                if let Some(inventory) = inventory
                                    && let Some(slots) = inventory.slots()
                                    && !slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                                    && !bot.tab_list().into_iter().any(|entry| {
                                        entry
                                            .1
                                            .display_name
                                            .unwrap_or_default()
                                            .to_string()
                                            .to_lowercase()
                                            .contains("[CoralProxy]")
                                    })
                                {
                                    bot.chat("/hub");
                                    join_bedwars_lobby(&bot).await;
                                }
                            }
                        });
                        break eprintln!("successfully joined the bedwars lobby...");
                    }

                    let mut inventory: Option<ContainerHandle> = None;
                    _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        if let Some(opened_inv) = bot.open_inventory() {
                            inventory.replace(opened_inv);
                        }
                    }));

                    if let Some(inventory) = inventory
                        && let Some(slots) = inventory.slots()
                        && slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                    {
                        let mut location = state.location.lock().await;
                        *location = BotLocation::BedwarsLobby;
                    }
                    if state.location.lock().await.eq(&BotLocation::Lobby) {
                        join_bedwars_lobby(&bot).await;
                    }
                }
            });
        }
        Event::Chat(message) => {
            handle_message_event(&bot, message, &state).await;
        }
        Event::Tick => {
            handle_instruction(&bot, &state).await;
        }
        _ => {}
    }

    Ok(())
}

async fn handle_instruction(bot: &Client, state: &State) {
    let mut guard = state.recv_instruction.lock().await;
    if let Some(recv) = guard.as_mut()
        && let Some(instruction) = recv.recv().await
    {
        eprintln!("received instruction");
        match instruction {
            Instruction::Chat(message) => bot.chat(message),
            Instruction::Jump => bot.jump(),
            Instruction::TabList(sender) => send_tab_list(bot, sender).await,
            Instruction::Walk(direction) => bot.walk(direction),
            Instruction::Sneak(sneaking) => bot.set_crouching(sneaking),
            Instruction::Slot(slot) => bot.set_selected_hotbar_slot(slot),
            Instruction::GetSlot(sender) => {
                let _ = sender.send(bot.selected_hotbar_slot()).await;
            }
        }
    }
}

async fn send_tab_list(bot: &Client, sender: Sender<String>) {
    let mut players = String::new();
    for (_, player) in bot.tab_list() {
        _ = writeln!(
            &mut players,
            "{}",
            player.display_name.unwrap_or_default().to_html()
        );
    }
    match sender.send(players).await {
        Ok(()) => eprintln!("successfully sent tablist..."),
        Err(_) => eprintln!("failed to send tablist..."),
    }
}

async fn join_bedwars_lobby(bot: &Client) {
    bot.set_selected_hotbar_slot(4);
    tokio::time::sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;

    eprintln!("opening lobby selector...");
    bot.right_click();
    tokio::time::sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;
    if let Some(container) = bot.wait_for_container_open(Some(400)).await
        && let Some(slots) = container.slots()
    {
        let Some(bed_idx) = slots
            .iter()
            .enumerate()
            .find(|(_idx, item)| item.kind().eq(&ItemKind::RedBed))
            .map(|v| v.0)
        else {
            return eprintln!("couldn't get the bed idx");
        };
        tokio::time::sleep(Duration::from_millis(1000)).await;
        container.left_click(bed_idx);
        bot.set_selected_hotbar_slot(3);
    } else {
        eprintln!("failed to get open container...");
    }
}

async fn handle_message_event(bot: &Client, message: azalea::chat::ChatPacket, state: &State) {
    println!("{}", message.message().to_ansi());
    handle_register_login(bot, &message, state).await;
    let html_message = message.message().to_html();
    {
        let mut messages = state.messages.lock().await;
        messages.push(html_message.clone());
    }
    if let Some(ref broadcaster) = state.broadcaster {
        broadcaster.send(WsServerMessage::NewMessage(html_message));
    }
    let message = message.message().to_string();
    if message.contains("La partita inizia tra") {
        bot.chat("/l");
    }
    if state.location.lock().await.eq(&BotLocation::BedwarsLobby)
        && message.to_lowercase().trim().starts_with('[')
        && let Some(mut message) = Message::parse(&message)
        && let Some(conn) = state.conn.lock().await.as_ref()
    {
        let username = message.name.as_str();
        if let Some(player_uuid) = bot.player_uuid_by_username(username)
            && let Some(player) = bot.tab_list().get(&player_uuid)
        {
            message = message.set_ping(player.latency.into());
        }
        match message.insert(conn) {
            Ok(()) => {}
            Err(_) => eprintln!("failed writing to db..."),
        }
    }
}

async fn handle_register_login(bot: &Client, message: &azalea::chat::ChatPacket, state: &State) {
    if message
        .message()
        .to_string()
        .trim()
        .contains("CORALMC » Per favore, esegui la registrazione con il comando: /register [password] [confermaPassword]")
    {
        eprintln!("registering...");
        bot.chat(format!("/register {PWD} {PWD}"));
    }
    if message
        .message()
        .to_string()
        .trim()
        .contains("CORALMC » Per favore, esegui l'autenticazione con il comando: /login [password]")
    {
        eprintln!("logging in...");
        bot.chat(format!("/login {PWD}"));
    }

    if message
        .message()
        .to_string()
        .trim()
        .contains("Login eseguito con successo.")
    {
        eprintln!("setting location to lobby...");
        let mut location = state.location.lock().await;
        *location = BotLocation::Lobby;
    }
}

pub trait RightClick {
    fn right_click(&self);
}

impl RightClick for Client {
    fn right_click(&self) {
        self.write_packet(ServerboundGamePacket::UseItem(
            azalea::protocol::packets::game::ServerboundUseItem {
                hand: azalea::protocol::packets::game::s_interact::InteractionHand::MainHand,
                seq: 0,
                y_rot: 0.0,
                x_rot: 0.0,
            },
        ));
    }
}
