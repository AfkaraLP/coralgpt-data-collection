use std::{sync::Arc, time::Duration};

use azalea::{
    WalkDirection, prelude::*, protocol::packets::game::ServerboundGamePacket,
    registry::builtin::ItemKind,
};
use tokio::{sync::mpsc::Sender, time};

use crate::{PWD, State, message::Message};
const REGISTER_REQUEST: &str = "CORALMC » Per favore, esegui la registrazione con il comando: /register [password] [confermaPassword]";
const LOGIN_REQUEST: &str =
    "CORALMC » Per favore, esegui l'autenticazione con il comando: /login [password]";
const LOGIN_SUCCESS: &str = "Login eseguito con successo.";

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
                let mut interval = time::interval(Duration::from_secs(1));
                let state = Arc::new(state);
                let bot = Arc::new(bot);

                loop {
                    interval.tick().await;
                    let state = state.clone();
                    let is_in_bedwars_lobby = {
                        let location = state.location.lock().await;
                        location.eq(&BotLocation::BedwarsLobby)
                    };
                    let new_bot = bot.clone();
                    if is_in_bedwars_lobby {
                        tokio::spawn(async move {
                            new_bot.walk(azalea::WalkDirection::Backward);
                            let mut interval = time::interval(Duration::from_secs(10));
                            loop {
                                interval.tick().await;
                                new_bot.jump();
                            }
                        });
                        break eprintln!("successfully joined the bedwars lobby...");
                    }
                    if let Some(inventory) = bot.open_inventory()
                        && let Some(slots) = inventory.slots()
                        && slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                    {
                        {
                            let mut location = state.location.lock().await;
                            *location = BotLocation::BedwarsLobby;
                        }
                    }
                    join_bedwars_lobby(bot.clone(), state).await;
                }
            });
        }
        Event::Chat(message) => {
            handle_message_event(&bot, message, state.clone()).await;
        }
        Event::Tick => {
            let mut guard = state.recv_instruction.lock().await;
            if let Some(recv) = guard.as_mut()
                && let Some(instruction) = recv.recv().await
            {
                eprintln!("received instruction");
                match instruction {
                    Instruction::Chat(message) => bot.chat(message),
                    Instruction::Jump => bot.jump(),
                    Instruction::TabList(sender) => {
                        let mut players = String::new();
                        eprintln!("tablist:");
                        for (_, player) in bot.tab_list().into_iter() {
                            eprintln!("{}", player.display_name.clone().unwrap_or_default());
                            players.push_str(&format!(
                                "{}\n",
                                player.display_name.unwrap_or_default()
                            ));
                        }
                        match sender.send(players).await {
                            Ok(_) => eprintln!("successfully sent tablist..."),
                            Err(_) => eprintln!("failed to send tablist..."),
                        }
                    }
                    Instruction::Walk(direction) => bot.walk(direction),
                    Instruction::Sneak(sneaking) => bot.set_crouching(sneaking),
                    Instruction::Slot(slot) => bot.set_selected_hotbar_slot(slot),
                    Instruction::GetSlot(sender) => {
                        let _ = sender.send(bot.selected_hotbar_slot()).await;
                    }
                }
            }
        }
        _ => {}
    }

    Ok(())
}

async fn join_bedwars_lobby(bot: Arc<Client>, state: Arc<State>) {
    if state.location.lock().await.eq(&BotLocation::Lobby) && !*state.going_to_bedwars.lock().await
    {
        let new_state = state.clone();
        tokio::spawn(async move {
            let mut going_to_bedwars = new_state.going_to_bedwars.lock().await;
            *going_to_bedwars = true;
        });
        bot.set_selected_hotbar_slot(4);
        bot.wait_updates(1).await;

        eprintln!("opening lobby selector...");
        bot.write_packet(ServerboundGamePacket::UseItem(
            azalea::protocol::packets::game::ServerboundUseItem {
                hand: azalea::protocol::packets::game::s_interact::InteractionHand::MainHand,
                seq: 0,
                y_rot: 0.0,
                x_rot: 0.0,
            },
        ));
        bot.wait_updates(1).await;
        // eprintln!("held item: {:#?}", bot.get_held_item());
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
            tokio::time::sleep(Duration::from_millis(500)).await;
            container.left_click(bed_idx);
            bot.set_selected_hotbar_slot(3);
        } else {
            eprintln!("failed to get open container...");
        }
        tokio::spawn(async move {
            let mut going_to_bedwars = state.going_to_bedwars.lock().await;
            *going_to_bedwars = false;
        });
    }
}

async fn handle_message_event(bot: &Client, message: azalea::chat::ChatPacket, state: State) {
    println!("{}", message.message().to_ansi());
    handle_register_login(bot, &message, &state).await;
    {
        let mut messages = state.messages.lock().await;
        messages.push(message.message().to_html());
    }
    let message = message.message().to_string();
    if state.location.lock().await.eq(&BotLocation::BedwarsLobby)
        && message.to_lowercase().trim().starts_with('[')
    {
        if let Some(mut message) = Message::parse(&message)
            && let Some(conn) = state.conn.lock().await.as_ref()
        {
            let username = message.name.as_str();
            if let Some(player_uuid) = bot.player_uuid_by_username(username)
                && let Some(player) = bot.tab_list().get(&player_uuid)
            {
                message = message.set_ping(player.latency as i64);
            }
            match message.insert(conn) {
                Ok(_) => {}
                Err(_) => eprintln!("failed writing to db..."),
            };
        }
    }
}

async fn handle_register_login(bot: &Client, message: &azalea::chat::ChatPacket, state: &State) {
    if message
        .message()
        .to_string()
        .trim()
        .contains(REGISTER_REQUEST)
    {
        eprintln!("registering...");
        bot.chat(format!("/register {PWD} {PWD}"));
    }
    if message.message().to_string().trim().contains(LOGIN_REQUEST) {
        eprintln!("logging in...");
        bot.chat(format!("/login {PWD}"));
    }

    if message.message().to_string().trim().contains(LOGIN_SUCCESS) {
        eprintln!("setting location to lobby...");
        let mut location = state.location.lock().await;
        *location = BotLocation::Lobby;
    }
}
