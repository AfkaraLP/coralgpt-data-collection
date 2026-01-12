use actix_web::{HttpResponse, Responder, get, post, web};
use std::sync::Arc;
use tokio::sync::{
    Mutex,
    mpsc::{self, Sender},
};

use crate::{bot::Instruction, buffer::FifoBuffer};

pub struct MessageLog {
    pub messages: Arc<Mutex<FifoBuffer<1000, String>>>,
}

const HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Server Chat</title>
    <link href="https://fonts.googleapis.com/css2?family=VT323&display=swap" rel="stylesheet">
    <style>
        :root {
            --dirt: #866043;
            --dirt-dark: #593D29;
            --stone: #7F7F7F;
            --stone-dark: #4A4A4A;
            --stone-light: #A0A0A0;
            --grass: #5D9B47;
            --grass-dark: #3E6B2F;
            --obsidian: #1D1D21;
            --bedrock: #0C0C0F;
            --gold: #FCDB05;
            --emerald: #17D14F;
            --diamond: #4AEDD9;
            --redstone: #D73131;
            --text: #FFFFFF;
            --text-shadow: #3F3F3F;
            --panel-bg: rgba(0,0,0,0.75);
        }
        @font-face {
            font-family: 'Minecraft';
            src: local('VT323'), local('Courier New');
        }
        * { box-sizing: border-box; margin: 0; padding: 0; }
        html, body {
            height: 100%;
            overflow: hidden;
        }
        body {
            font-family: 'VT323', 'Courier New', monospace;
            background: var(--bedrock);
            background-image:
                repeating-linear-gradient(0deg, transparent, transparent 2px, rgba(255,255,255,0.03) 2px, rgba(255,255,255,0.03) 4px),
                repeating-linear-gradient(90deg, transparent, transparent 2px, rgba(255,255,255,0.03) 2px, rgba(255,255,255,0.03) 4px);
            color: var(--text);
            display: flex;
            font-size: 18px;
            image-rendering: pixelated;
        }

        /* Minecraft Button Style */
        .mc-btn {
            background: linear-gradient(180deg, #7A7A7A 0%, #5A5A5A 50%, #4A4A4A 50%, #3A3A3A 100%);
            border: 3px solid;
            border-color: #FFFFFF #555555 #555555 #FFFFFF;
            color: var(--text);
            padding: 8px 16px;
            font-family: inherit;
            font-size: 18px;
            cursor: pointer;
            text-shadow: 2px 2px 0 var(--text-shadow);
            transition: filter 0.1s;
            width: 100%;
            margin-bottom: 8px;
        }
        .mc-btn:hover {
            background: linear-gradient(180deg, #9A9A9A 0%, #7A7A7A 50%, #6A6A6A 50%, #5A5A5A 100%);
        }
        .mc-btn:active {
            border-color: #555555 #FFFFFF #FFFFFF #555555;
            padding: 9px 15px 7px 17px;
        }
        .mc-btn-green {
            background: linear-gradient(180deg, #5D9B47 0%, #4A8039 50%, #3E6B2F 50%, #2E5B1F 100%);
        }
        .mc-btn-green:hover {
            background: linear-gradient(180deg, #7DBB67 0%, #5D9B47 50%, #4A8039 50%, #3E6B2F 100%);
        }

        /* Sidebar */
        .sidebar {
            width: 280px;
            background: var(--panel-bg);
            border-right: 4px solid var(--stone-dark);
            display: flex;
            flex-direction: column;
            flex-shrink: 0;
        }
        .sidebar-header {
            padding: 24px 20px;
            border-bottom: 4px solid var(--stone-dark);
            background: linear-gradient(180deg, var(--stone-dark) 0%, var(--obsidian) 100%);
        }
        .logo {
            font-size: 24px;
            text-shadow: 2px 2px 0 #000;
            color: var(--diamond);
        }
        .logo-sub {
            font-size: 14px;
            color: var(--stone-light);
            text-shadow: 1px 1px 0 #000;
            margin-top: 4px;
        }

        /* Controls */
        .controls {
            padding: 16px;
            flex: 1;
        }
        .control-section {
            margin-bottom: 16px;
        }
        .control-label {
            font-size: 16px;
            color: var(--stone-light);
            margin-bottom: 8px;
            text-shadow: 1px 1px 0 #000;
        }

        /* Players Panel */
        .players-panel {
            border-top: 4px solid var(--stone-dark);
            padding: 16px;
            max-height: 250px;
            overflow: hidden;
            display: flex;
            flex-direction: column;
            background: linear-gradient(180deg, rgba(0,0,0,0.3) 0%, rgba(0,0,0,0.5) 100%);
        }
        .players-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 12px;
        }
        .players-title {
            font-size: 18px;
            color: var(--emerald);
            text-shadow: 2px 2px 0 #000;
        }
        .players-count {
            font-size: 18px;
            color: var(--gold);
            background: rgba(0,0,0,0.5);
            padding: 2px 10px;
            border: 2px solid var(--stone-dark);
        }
        .players-list {
            overflow-y: auto;
            flex: 1;
        }
        .player-item {
            padding: 4px 8px;
            font-size: 16px;
            color: var(--text);
            text-shadow: 1px 1px 0 #000;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }
        .player-item:hover {
            background: rgba(255,255,255,0.1);
        }

        /* Main Chat Area */
        .main {
            flex: 1;
            display: flex;
            flex-direction: column;
            min-width: 0;
            min-height: 0;
            overflow: hidden;
            background:
                linear-gradient(180deg, rgba(0,0,0,0.8) 0%, rgba(0,0,0,0.6) 100%),
                repeating-linear-gradient(
                    0deg,
                    var(--obsidian) 0px,
                    var(--obsidian) 32px,
                    #252528 32px,
                    #252528 64px
                );
        }
        .chat-header {
            padding: 16px 24px;
            border-bottom: 4px solid var(--stone-dark);
            background: linear-gradient(180deg, var(--stone-dark) 0%, var(--obsidian) 100%);
            display: flex;
            align-items: center;
            justify-content: space-between;
        }
        .chat-title {
            font-size: 24px;
            color: var(--diamond);
            text-shadow: 2px 2px 0 #000;
        }
        .chat-subtitle {
            font-size: 16px;
            color: var(--stone-light);
            text-shadow: 1px 1px 0 #000;
        }

        /* Messages Area */
        .messages {
            flex: 1;
            min-height: 0;
            overflow-y: auto;
            padding: 16px;
            display: flex;
            flex-direction: column;
            gap: 2px;
        }
        .messages-empty {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            color: var(--stone-light);
            font-size: 20px;
            text-shadow: 2px 2px 0 #000;
        }
        .message {
            max-width: 85%;
            padding: 6px 12px;
            background: rgba(0,0,0,0.5);
            border-left: 3px solid var(--stone);
            font-size: 17px;
            text-shadow: 1px 1px 0 #000;
            word-wrap: break-word;
        }
        .message:hover {
            background: rgba(255,255,255,0.1);
        }
        .message-received {
            align-self: flex-start;
        }
        .message-bubble {
            color: var(--text);
        }

        /* Input Area */
        .input-area {
            padding: 16px;
            border-top: 4px solid var(--stone-dark);
            background: linear-gradient(180deg, var(--obsidian) 0%, var(--bedrock) 100%);
        }
        .input-container {
            display: flex;
            gap: 12px;
            align-items: stretch;
        }
        .input-wrapper {
            flex: 1;
            background: #000;
            border: 3px solid;
            border-color: #555555 #FFFFFF #FFFFFF #555555;
            padding: 2px;
        }
        .input-wrapper:focus-within {
            border-color: var(--emerald) var(--grass-dark) var(--grass-dark) var(--emerald);
        }
        .chat-input {
            width: 100%;
            background: #000;
            border: none;
            padding: 10px 12px;
            color: var(--text);
            font-size: 18px;
            font-family: inherit;
            outline: none;
        }
        .chat-input::placeholder {
            color: var(--stone);
        }
        .send-btn {
            padding: 10px 20px;
            background: linear-gradient(180deg, #5D9B47 0%, #4A8039 50%, #3E6B2F 50%, #2E5B1F 100%);
            border: 3px solid;
            border-color: #FFFFFF #555555 #555555 #FFFFFF;
            cursor: pointer;
            font-family: inherit;
            font-size: 18px;
            color: var(--text);
            text-shadow: 2px 2px 0 var(--text-shadow);
        }
        .send-btn:hover {
            background: linear-gradient(180deg, #7DBB67 0%, #5D9B47 50%, #4A8039 50%, #3E6B2F 100%);
        }
        .send-btn:active {
            border-color: #555555 #FFFFFF #FFFFFF #555555;
        }

        /* Toast */
        .toast {
            position: fixed;
            bottom: 100px;
            left: 50%;
            transform: translateX(-50%) translateY(20px);
            background: rgba(0,0,0,0.9);
            border: 3px solid var(--stone);
            padding: 10px 20px;
            font-size: 18px;
            text-shadow: 2px 2px 0 #000;
            opacity: 0;
            transition: all 0.3s ease;
            pointer-events: none;
            z-index: 100;
        }
        .toast.show {
            opacity: 1;
            transform: translateX(-50%) translateY(0);
        }
        .toast.success { border-color: var(--emerald); color: var(--emerald); }
        .toast.error { border-color: var(--redstone); color: var(--redstone); }

        /* Scrollbar - Minecraft style */
        ::-webkit-scrollbar { width: 12px; }
        ::-webkit-scrollbar-track { background: var(--obsidian); border: 2px solid var(--stone-dark); }
        ::-webkit-scrollbar-thumb {
            background: linear-gradient(180deg, var(--stone) 0%, var(--stone-dark) 100%);
            border: 2px solid;
            border-color: var(--stone-light) var(--stone-dark) var(--stone-dark) var(--stone-light);
        }
        ::-webkit-scrollbar-thumb:hover {
            background: linear-gradient(180deg, var(--stone-light) 0%, var(--stone) 100%);
        }
    </style>
</head>
<body>
    <aside class="sidebar">
        <div class="controls">
            <div class="control-section">
                <div class="control-label">Bot Actions</div>
                <button class="mc-btn" onclick="sendJump()">Jump</button>
            </div>
        </div>

        <div class="players-panel">
            <div class="players-header">
                <span class="players-title">Online Players</span>
                <span class="players-count" id="playerCount">0</span>
            </div>
            <div class="players-list" id="playersList">
                <div class="player-item" style="color: var(--stone);">Loading...</div>
            </div>
        </div>
    </aside>

    <main class="main">
        <header class="chat-header">
            <div>
                <div class="chat-title">Server Chat</div>
                <div class="chat-subtitle">wind.coralmc.it</div>
            </div>
        </header>

        <div class="messages" id="messages">
            <div class="messages-empty">
                <span>Loading chat history...</span>
            </div>
        </div>

        <div class="input-area">
            <div class="input-container">
                <div class="input-wrapper">
                    <input type="text" class="chat-input" id="chatInput" placeholder="Type a message..." autocomplete="off">
                </div>
                <button class="send-btn" onclick="sendChat()">Send</button>
            </div>
        </div>
    </main>

    <div class="toast" id="toast"></div>

    <script>
        const messagesEl = document.getElementById('messages');
        const inputEl = document.getElementById('chatInput');
        const toast = document.getElementById('toast');
        let lastMessageCount = 0;
        let adFilters = [];

        async function loadFilters() {
            try {
                const res = await fetch('https://raw.githubusercontent.com/AfkaraLP/no-coral-ad/main/base.txt');
                const text = await res.text();
                adFilters = text.split('\n').map(s => s.trim()).filter(s => s);
            } catch (e) {
                console.error('Failed to load filters');
            }
        }

        function isFiltered(msg) {
            return adFilters.some(filter => msg.includes(filter));
        }

        loadFilters();
        setInterval(loadFilters, 60000);

        function showToast(message, type = 'info') {
            toast.textContent = message;
            toast.className = 'toast show ' + type;
            setTimeout(() => toast.classList.remove('show'), 2500);
        }

        function escapeHtml(text) {
            const div = document.createElement('div');
            div.textContent = text;
            return div.innerHTML;
        }

        async function loadMessages() {
            try {
                const res = await fetch('/messages');
                const text = await res.text();
                const messages = text.split('\n').filter(m => m.trim());

                if (messages.length !== lastMessageCount) {
                    lastMessageCount = messages.length;
                    renderMessages(messages);
                }
            } catch (e) {
                // Silently fail
            }
        }

        function renderMessages(messages) {
            const filtered = messages.filter(msg => !isFiltered(msg));

            if (filtered.length === 0) {
                messagesEl.innerHTML = '<div class="messages-empty"><span>No messages yet</span></div>';
                return;
            }

            const wasAtBottom = messagesEl.scrollHeight - messagesEl.scrollTop <= messagesEl.clientHeight + 50;

            messagesEl.innerHTML = filtered.slice().reverse().map(msg =>
                `<div class="message message-received"><div class="message-bubble">${escapeHtml(msg)}</div></div>`
            ).join('');

            if (wasAtBottom) {
                messagesEl.scrollTop = messagesEl.scrollHeight;
            }
        }

        async function sendChat() {
            const msg = inputEl.value.trim();
            if (!msg) return;

            inputEl.value = '';

            try {
                const res = await fetch('/chat', { method: 'POST', body: msg });
                const result = await res.text();
                if (result.includes('success')) {
                    showToast('Message sent', 'success');
                    setTimeout(loadMessages, 500);
                } else {
                    showToast('Failed to send', 'error');
                }
            } catch (e) {
                showToast('Error: ' + e.message, 'error');
            }
        }

        async function getTab() {
            try {
                const res = await fetch('/tab', { method: 'POST' });
                const text = await res.text();
                const players = text.split('\n').filter(p => p.trim());

                document.getElementById('playerCount').textContent = players.length;
                const list = document.getElementById('playersList');

                if (players.length === 0 || text.includes('failed')) {
                    list.innerHTML = '<div class="player-item" style="color: var(--text-muted);">No players online</div>';
                } else {
                    list.innerHTML = players.map(p => `
                        <div class="player-item">${escapeHtml(p)}</div>
                    `).join('');
                }
            } catch (e) {
                // Silently fail
            }
        }

        async function sendJump() {
            try {
                const res = await fetch('/jump', { method: 'POST' });
                const result = await res.text();
                if (result.includes('success')) {
                    showToast('Jump!', 'success');
                } else {
                    showToast('Failed', 'error');
                }
            } catch (e) {
                showToast('Error: ' + e.message, 'error');
            }
        }

        inputEl.addEventListener('keypress', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                sendChat();
            }
        });

        // Initial load and auto-refresh
        loadMessages();
        getTab();
        setInterval(loadMessages, 500);
        setInterval(getTab, 5000);
    </script>
</body>
</html>
"#;

#[get("/")]
pub async fn index() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html")
        .body(HTML)
}

#[post("/chat")]
pub async fn bot_chat(
    req_body: String,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    match data.send(Instruction::Chat(req_body)).await {
        Ok(_) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/jump")]
pub async fn bot_jump(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    match data.send(Instruction::Jump).await {
        Ok(_) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/tab")]
pub async fn bot_tab(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    match data.send(Instruction::TabList(tx)).await {
        Ok(_) => match rx.recv().await {
            Some(tablist) => tablist,
            None => "failed receiving tablist".to_string(),
        },
        Err(_) => "failed sending tab instruction".to_string(),
    }
}

#[get("/messages")]
pub async fn bot_messages(data: web::Data<MessageLog>) -> impl Responder {
    let messages = data.messages.lock().await;
    messages
        .iter()
        .map(|msg| format!("{msg}\n"))
        .collect::<String>()
}
