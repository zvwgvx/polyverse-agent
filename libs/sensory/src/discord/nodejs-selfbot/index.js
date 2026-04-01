const { Client } = require('discord.js-selfbot-v13');
const WebSocket = require('ws');
const path = require('path');
require('dotenv').config({ path: path.resolve(__dirname, '../../../../../.env') });

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
let outboundSendChain = Promise.resolve();
const recentOutbound = new Map();
const OUTBOUND_ECHO_TTL_MS = 15000;

function rememberOutbound(channelId, content) {
    const normalized = `${channelId}::${String(content || '').trim()}`;
    recentOutbound.set(normalized, Date.now());
    setTimeout(() => recentOutbound.delete(normalized), OUTBOUND_ECHO_TTL_MS).unref?.();
}

function isRecentOutboundEcho(channelId, content) {
    const normalized = `${channelId}::${String(content || '').trim()}`;
    const seenAt = recentOutbound.get(normalized);
    return typeof seenAt === 'number' && (Date.now() - seenAt) <= OUTBOUND_ECHO_TTL_MS;
}

const client = new Client({
    checkUpdate: false,
});

function connectWebSocket() {
    ws = new WebSocket(WS_URL, { perMessageDeflate: false });

    ws.on('open', () => {
        console.log(`[Selfbot] Connected to Core WebSocket at ${WS_URL}`);
        wsConnected = true;
        consecutiveConnectionFailures = 0;
    });

    ws.on('message', async (data) => {
        try {
            const payload = JSON.parse(data);
            if (payload.type === 'response') {
                enqueueOutgoingResponse(payload.data);
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

function enqueueOutgoingResponse(data) {
    outboundSendChain = outboundSendChain
        .catch(() => {})
        .then(async () => {
            const { channel_id, content, reply_to_message_id, is_typing } = data;
            const channel = await client.channels.fetch(channel_id).catch(() => null);

            if (!channel) {
                console.error(`[Selfbot] Unknown channel ID: ${channel_id}`);
                return;
            }

            if (is_typing) {
                await channel.sendTyping();
                return;
            }

            if (reply_to_message_id && channel.type !== 'DM') {
                try {
                    const message = await channel.messages.fetch(reply_to_message_id);
                    const sent = await message.reply(content);
                    rememberOutbound(channel_id, sent?.content ?? content);
                    console.log(`[Selfbot] Replied to ${reply_to_message_id} in ${channel_id}`);
                    return;
                } catch (error) {
                    console.error(`[Selfbot] Could not reply to ${reply_to_message_id}:`, error.message);
                }
            }

            const sent = await channel.send(content);
            rememberOutbound(channel_id, sent?.content ?? content);
            console.log(`[Selfbot] Sent message to ${channel_id}`);
        })
        .catch((error) => {
            console.error('[Selfbot] Failed to send queued response:', error.message);
        });
}

client.on('ready', async () => {
    console.log(`[Selfbot] Connected to Discord as ${client.user.tag}`);
    connectWebSocket();
});

client.on('messageCreate', async (msg) => {
    const isAllowedChannel = msg.channelId === '1410283966992351363';
    const isAllowedDm = !msg.guildId && msg.author.id === '1320303839701897230';

    if (!isAllowedChannel && !isAllowedDm) {
        return;
    }

    if (msg.author.id === client.user.id) {
        return;
    }

    if (!wsConnected || ws.readyState !== WebSocket.OPEN) return;
    if (isRecentOutboundEcho(msg.channelId, msg.content)) {
        return;
    }

    let isMention = false;
    let isDm = !msg.guildId;
    let isReplyToSelf = false;

    const hasExplicitMention = msg.content.includes(`<@${client.user.id}>`) ||
        msg.content.includes(`<@!${client.user.id}>`);

    if (!isDm && msg.reference?.messageId) {
        try {
            const referenced = await msg.fetchReference();
            isReplyToSelf = referenced?.author?.id === client.user.id;
        } catch (error) {
            console.warn(`[Selfbot] Failed to resolve reply reference for ${msg.id}: ${error.message}`);
        }
    }

    if (hasExplicitMention || isDm || isReplyToSelf) {
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
