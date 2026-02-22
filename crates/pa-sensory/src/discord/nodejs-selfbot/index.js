const { Client } = require('discord.js-selfbot-v13');
const WebSocket = require('ws');
const path = require('path');
require('dotenv').config({ path: path.resolve(__dirname, '../../../../../.env') }); // Reaches back to project root

const DISCORD_SELFBOT_TOKEN = process.env.DISCORD_SELFBOT_TOKEN;
if (!DISCORD_SELFBOT_TOKEN) {
    console.warn("DISCORD_SELFBOT_TOKEN is not defined in .env! Exiting node process.");
    process.exit(1);
}

const WS_URL = 'ws://127.0.0.1:9000';
let ws = null;
let wsConnected = false;
let consecutiveConnectionFailures = 0;
const MAX_FAILURES = 3;

const client = new Client({
    checkUpdate: false, // Don't check for selfbot updates on startup
});

function connectWebSocket() {
    // Tối ưu hoá cực mạnh: Tắt nén tin nhắn (perMessageDeflate: false) để gửi packet ngay lập tức không qua tầng zlib
    ws = new WebSocket(WS_URL, { perMessageDeflate: false });

    ws.on('open', () => {
        console.log(`[Selfbot] Connected to Core WebSocket at ${WS_URL}`);
        wsConnected = true;
        consecutiveConnectionFailures = 0; // Reset counter on successful connection
    });

    ws.on('message', async (data) => {
        try {
            const payload = JSON.parse(data);
            if (payload.type === 'response') {
                const { channel_id, content, reply_to_message_id, is_typing } = payload.data;
                const channel = await client.channels.fetch(channel_id).catch(() => null);

                if (channel) {
                    if (is_typing) {
                        await channel.sendTyping();
                        return;
                    }

                    if (reply_to_message_id && channel.type !== 'DM') {
                        try {
                            const message = await channel.messages.fetch(reply_to_message_id);
                            await message.reply(content);
                            console.log(`[Selfbot] Replied to ${reply_to_message_id} in ${channel_id}`);
                        } catch (e) {
                            console.error(`[Selfbot] Could not reply to ${reply_to_message_id}:`, e.message);
                            await channel.send(content); // fallback if message not found
                        }
                    } else {
                        await channel.send(content);
                        console.log(`[Selfbot] Sent message to ${channel_id}`);
                    }
                } else {
                    console.error(`[Selfbot] Unknown channel ID: ${channel_id}`);
                }
            }
        } catch (error) {
            console.error('[Selfbot] WS Message parsing failed:', error);
        }
    });

    ws.on('close', () => {
        console.log('[Selfbot] WebSocket disconnected. Reconnecting in 3s...');
        wsConnected = false;
        setTimeout(connectWebSocket, 3000);
    });

    ws.on('error', (err) => {
        console.error('[Selfbot] WebSocket error:', err.message);
        if (err.message.includes('ECONNREFUSED')) {
            consecutiveConnectionFailures++;
            if (consecutiveConnectionFailures >= MAX_FAILURES) {
                console.error(`[Selfbot] Failed to connect to Core ${MAX_FAILURES} times. Parent Rust process is likely dead. Self-terminating.`);
                process.exit(0);
            }
        }
        ws.close();
    });
}

client.on('ready', async () => {
    console.log(`[Selfbot] Connected to Discord as ${client.user.tag}`);
    connectWebSocket();
});

client.on('messageCreate', async (msg) => {
    // Only process messages in the specified channel OR direct messages from a specific user
    const isAllowedChannel = msg.channelId === '1410283966992351363';
    const isAllowedDm = !msg.guildId && msg.author.id === '1320303839701897230';

    if (!isAllowedChannel && !isAllowedDm) {
        return;
    }

    // Ignore our own messages to prevent infinite loops
    if (msg.author.id === client.user.id) {
        return;
    }

    if (!wsConnected || ws.readyState !== WebSocket.OPEN) return;

    let isMention = false;
    let isDm = !msg.guildId;

    if (msg.mentions.users.has(client.user.id) || isDm) {
        isMention = true;
    }

    const payload = {
        type: 'message',
        data: {
            platform: 'DiscordSelfbot',
            channel_id: msg.channelId,
            message_id: msg.id,
            user_id: msg.author.id,
            username: msg.author.username,
            content: msg.content,
            is_mention: isMention,
            is_dm: isDm
        }
    };

    ws.send(JSON.stringify(payload));
});

client.login(DISCORD_SELFBOT_TOKEN).catch(e => {
    console.error(`[Selfbot] Failed to login: ${e.message}`);
    process.exit(1);
});
