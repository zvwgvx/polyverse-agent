const { Client } = require('discord.js-selfbot-v13');
const net = require('net');
const path = require('path');
require('dotenv').config({ path: path.resolve(__dirname, '../../../../../.env') });

const DISCORD_SELFBOT_TOKEN = process.env.DISCORD_SELFBOT_TOKEN;
if (!DISCORD_SELFBOT_TOKEN) {
    console.warn("DISCORD_SELFBOT_TOKEN is not defined in .env! Exiting node process.");
    process.exit(1);
}

const SOCKET_PATH = process.env.PLATFORM_RELAY_SOCKET || '/tmp/polyverse-agent-relay.sock';
let clientSocket = null;
let connected = false;
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

function sendToRelay(msgObject) {
    if (!connected || !clientSocket) return;
    const jsonStr = JSON.stringify(msgObject);
    const buf = Buffer.from(jsonStr, 'utf8');
    const lenBuf = Buffer.alloc(4);
    lenBuf.writeUInt32LE(buf.length, 0);
    clientSocket.write(lenBuf);
    clientSocket.write(buf);
}

function connectUDS() {
    clientSocket = net.createConnection(SOCKET_PATH);
    let buffer = Buffer.alloc(0);

    clientSocket.on('connect', () => {
        console.log(`[Selfbot] Connected to Agent UDS at ${SOCKET_PATH}`);
        connected = true;
        consecutiveConnectionFailures = 0;

        // Initial ping
        sendToRelay({ type: "ping" });
    });

    clientSocket.on('data', (data) => {
        buffer = Buffer.concat([buffer, data]);
        while (buffer.length >= 4) {
            const msgLen = buffer.readUInt32LE(0);
            if (buffer.length < 4 + msgLen) {
                break; // Need more data
            }

            const msgBuf = buffer.slice(4, 4 + msgLen);
            buffer = buffer.slice(4 + msgLen);

            try {
                const payload = JSON.parse(msgBuf.toString('utf8'));
                if (payload.type === 'response') {
                    enqueueOutgoingResponse(payload.event);
                } else if (payload.type === 'pong') {
                    // console.log('[Selfbot] Pong received');
                } else if (payload.type === 'ack') {
                    // Message acknowledged
                }
            } catch (error) {
                console.error('[Selfbot] UDS Message parsing failed:', error);
            }
        }
    });

    clientSocket.on('close', () => {
        console.log('[Selfbot] UDS disconnected. Reconnecting in 3s...');
        connected = false;
        clientSocket = null;
        setTimeout(connectUDS, 3000);
    });

    clientSocket.on('error', (err) => {
        console.error('[Selfbot] UDS error:', err.message);
        if (err.message.includes('ENOENT') || err.message.includes('ECONNREFUSED')) {
            consecutiveConnectionFailures++;
            if (consecutiveConnectionFailures >= MAX_FAILURES) {
                console.error(`[Selfbot] Failed to connect to Agent ${MAX_FAILURES} times. Agent is likely dead. Exiting.`);
                process.exit(0);
            }
        }
    });
}

function enqueueOutgoingResponse(event) {
    outboundSendChain = outboundSendChain
        .catch(() => {})
        .then(async () => {
            const { channel_id, content, reply_to_message_id } = event;
            const channel = await client.channels.fetch(channel_id).catch(() => null);

            if (!channel) {
                console.error(`[Selfbot] Unknown channel ID: ${channel_id}`);
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
    connectUDS();
});

async function extractImageAttachments(msg) {
    const attachments = [];
    const candidates = Array.from(msg.attachments?.values?.() ?? []).slice(0, 4);

    for (const attachment of candidates) {
        const mimeType = attachment.contentType || '';
        if (!['image/jpeg', 'image/png', 'image/webp', 'image/gif'].includes(mimeType)) {
            continue;
        }
        if ((attachment.size || 0) > 5 * 1024 * 1024) {
            continue;
        }
        if (typeof fetch !== 'function') {
            continue;
        }

        try {
            const response = await fetch(attachment.url);
            if (!response.ok) {
                continue;
            }
            const buffer = Buffer.from(await response.arrayBuffer());
            if (buffer.length > 5 * 1024 * 1024) {
                continue;
            }
            attachments.push({
                mime_type: mimeType,
                filename: attachment.name || null,
                source_url: attachment.url,
                data_base64: buffer.toString('base64')
            });
        } catch (error) {
            console.warn(`[Selfbot] Failed to fetch image attachment ${attachment.name}: ${error.message}`);
        }
    }

    return attachments;
}

client.on('messageCreate', async (msg) => {
    const isAllowedChannel = msg.channelId === '1410283966992351363';
    const isAllowedDm = !msg.guildId && msg.author.id === '1320303839701897230';

    if (!isAllowedChannel && !isAllowedDm) {
        return;
    }

    if (msg.author.id === client.user.id) {
        return;
    }

    if (!connected) return;
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
        type: 'ingest',
        event: {
            platform: 'DiscordSelfbot',
            channel_id: msg.channelId,
            message_id: msg.id,
            user_id: msg.author.id,
            username: msg.author.username,
            content: msg.content,
            attachments: await extractImageAttachments(msg),
            is_mention: isMention,
            is_dm: isDm,
            timestamp: new Date().toISOString()
        }
    };

    sendToRelay(payload);
});

client.login(DISCORD_SELFBOT_TOKEN).catch(e => {
    console.error(`[Selfbot] Failed to login: ${e.message}`);
    process.exit(1);
});
