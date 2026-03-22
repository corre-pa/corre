# Milestone 6: Signal Integration (Future)

## Goal

Add Signal as a second messaging channel alongside Telegram. Users can interact with the
gym tracker assistant via Signal voice and text messages.

## Status

**Deferred.** This milestone is planned but not yet scheduled for implementation.
Signal integration is significantly more complex than Telegram due to the lack of an
official bot API.

## Why it's hard

1. **No official bot API.** Signal doesn't provide a bot platform. There are community
   tools that implement the Signal protocol, but they are reverse-engineered and can
   break on Signal app updates.

2. **Phone number requirement.** The bot needs a dedicated phone number to register
   on Signal. This phone number receives an SMS verification code.

3. **Java dependency.** The most mature tool (signal-cli) is a Java application,
   requiring a JVM in the Docker container (~200MB+ image size increase).

4. **Voice message support.** Signal voice messages are received as file attachments.
   The format and handling is less well-documented than Telegram's.

5. **End-to-end encryption.** Signal's E2E encryption means the bot needs to manage
   its own key pair and session state. This is handled by signal-cli but adds
   complexity to the setup.

## Approach: signal-cli as a sidecar

### signal-cli

[signal-cli](https://github.com/AsamK/signal-cli) is a Java-based command-line tool
that implements the Signal protocol. It can:
- Register a phone number
- Send and receive messages
- Receive attachments (including voice messages)
- Run as a daemon with a JSON-RPC API

### Architecture

```
crates/mcp-signal/          New MCP server wrapping signal-cli
  src/main.rs               rmcp server with send_message, get_messages tools

Docker sidecar:
  signal-cli daemon         Long-running process with JSON-RPC socket

crates/corre-gym/src/
  signal/
    mod.rs
    client.rs               Signal client (talks to signal-cli JSON-RPC)
    types.rs                Signal message types
  messaging/
    mod.rs                  Messaging abstraction trait
    telegram.rs             Telegram implementation (extract from M1)
    signal.rs               Signal implementation
```

### Messaging abstraction trait

Before adding Signal, refactor the Telegram-specific code behind a trait:

```rust
#[async_trait]
pub trait MessagingChannel: Send + Sync {
    /// Channel name ("telegram", "signal")
    fn name(&self) -> &str;

    /// Poll for new messages.
    async fn poll_messages(&self) -> Result<Vec<IncomingMessage>>;

    /// Send a text message.
    async fn send_text(&self, recipient: &str, text: &str) -> Result<()>;

    /// Send a voice message.
    async fn send_voice(&self, recipient: &str, audio_path: &Path) -> Result<()>;

    /// Download a voice message attachment.
    async fn download_voice(&self, file_ref: &str, dest: &Path) -> Result<()>;

    /// Send a typing/recording indicator.
    async fn send_activity(&self, recipient: &str, activity: Activity) -> Result<()>;
}

pub struct IncomingMessage {
    pub channel: String,          // "telegram" or "signal"
    pub sender_id: String,        // platform-specific user ID
    pub sender_name: String,
    pub text: Option<String>,
    pub voice_file_ref: Option<String>,
    pub timestamp: DateTime<Utc>,
}
```

### Main loop refactoring

The main polling loop becomes channel-agnostic:

```rust
// Poll all enabled channels concurrently
let mut channels: Vec<Box<dyn MessagingChannel>> = vec![];
if telegram_enabled { channels.push(Box::new(telegram_channel)); }
if signal_enabled { channels.push(Box::new(signal_channel)); }

loop {
    for channel in &channels {
        let messages = channel.poll_messages().await?;
        for msg in messages {
            let user = ensure_user_by_channel(&msg.channel, &msg.sender_id, &msg.sender_name)?;
            // Process through same assistant handler...
        }
    }
}
```

### signal-cli setup

```sh
# Register a phone number (one-time setup)
signal-cli -a +1234567890 register
# Enter verification code received via SMS
signal-cli -a +1234567890 verify 123456

# Run as daemon with JSON-RPC
signal-cli -a +1234567890 daemon --socket /var/run/signal-cli.sock
```

### Docker setup

```yaml
# docker-compose.yml addition
signal-cli:
  image: registry.gitlab.com/signald/signald:latest  # or build signal-cli image
  volumes:
    - signal-data:/var/lib/signal-cli
    - /var/run/signal-cli:/var/run/signal-cli
  restart: unless-stopped
  networks: [corre-internal]

volumes:
  signal-data:
```

### MCP server (optional)

If other Corre apps want to send Signal messages, create an MCP server:

```rust
// crates/mcp-signal/src/main.rs
// Similar to mcp-telegram but talks to signal-cli's JSON-RPC socket

#[tool(description = "Send a message via Signal")]
async fn send_message(&self, #[tool(aggr)] params: SignalMessageParams) -> Result<String, String> {
    // POST to signal-cli JSON-RPC socket
}
```

### Dashboard auth for Signal users

The Telegram Login Widget doesn't work for Signal users. Options:

1. **Magic link via Signal**: Send a one-time login link to the user's Signal number.
   User clicks it, server verifies the token, sets session cookie.

2. **Token-based**: Generate a persistent API token per user, displayed via Signal
   chat. User enters it on the login page.

3. **Dual auth**: Require both Telegram Login Widget AND Signal registration.
   Signal-only users would need the token-based approach.

Recommended: Magic link via Signal (most user-friendly).

## Risks

- **signal-cli stability**: Can break when Signal updates their protocol
- **Phone number management**: Need a dedicated phone number
- **JVM overhead**: ~200MB image size increase
- **Latency**: signal-cli may be slower than Telegram Bot API
- **Voice format**: Signal voice messages may use different codec than Telegram

## Prerequisites before starting

1. A dedicated phone number for the bot
2. signal-cli tested manually (register, send, receive)
3. Messaging abstraction trait designed and Telegram refactored behind it
4. Docker compose updated with signal-cli sidecar

## Estimation

This is the most uncertain milestone. The signal-cli integration may surface unexpected
issues. Budget extra time for debugging the Signal protocol layer.
