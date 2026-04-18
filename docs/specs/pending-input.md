# Pending Input with Stream Reference Confirmation

## Goal

When a user submits input, it should appear at the bottom of the log as "pending" — visually distinct from confirmed messages. The pending input stays there until a downstream actor (LLM test) confirms it has been consumed, at which point it joins the normal log.

This allows multiple UI frontends to all show the same pending state, and the confirmation propagates to all of them via the UI hub.

## Current Architecture

```
TUI (ui:user, ui:input)
  │
  ├─ publishes user input → ui:input
  │
  └─ subscribes to ui:user (hub forwards from subscribed topics)

UI Hub (default)
  ├─ subscribes to ui:input (receives TUI input)
  │   └─ routes to: llm-test:0:in
  │
  └─ subscribes to: sar:echo, sar:reverse, llm:0:stream
      └─ forwards to: ui:user

LlmTestActor
  ├─ subscribes to: llm-test:0:in (user input)
  ├─ subscribes to: llm:0:stream (stream chunks from LLM)
  ├─ subscribes to: llm:0:out (LLM final response)
  └─ publishes to: llm:0:in (forward to LLM)

LlmActor
  └─ publishes to: llm:0:stream (stream chunks)
```

## Message Types

### UserInput
- **Source:** TUI → `ui:input`
- **Payload:** the user's text
- **Meta:** none (TUI tracks pending locally)
- **Flow:** `ui:input` → hub → `llm-test:0:in`

### Stream Chunk
- **Source:** LlmActor → `llm:0:stream`
- **Payload:** plain text (a word or partial word)
- **Meta:** none
- **Flow:** `llm:0:stream` → TUI (direct) + hub → `ui:user`

### StreamEnd
- **Source:** LlmActor → `llm:0:stream`
- **Payload:** `{"type": "stream_end"}`
- **Meta:** none
- **Flow:** `llm:0:stream` → LlmTestActor (to detect end)

### StreamReference (confirmation)
- **Source:** LlmTestActor → `llm:0:stream`
- **Payload:** `{}`
- **Meta:** `{"type": "StreamReference", "reference": "<original_message_uuid>"}`
- **Flow:** `llm:0:stream` → hub → `ui:user` → all frontends

## Pending Input Lifecycle

```
1. TUI user types "Hello" and presses Enter
   TUI creates: pending_input = "Hello [ref: abc123]"
   TUI publishes to ui:input: { payload: "Hello", reference: "abc123" }

2. TUI renders: log_items + streaming_item + pending_input (at bottom)
   Display:
     [echo result]
     [streaming text...]
   > Hello [ref: abc123]

3. Hub routes to llm-test:0:in
   LlmTestActor stores: last_reference = "abc123"
   LlmTestActor forwards to llm:0:in

4. LlmActor streams chunks to llm:0:stream
   TUI receives chunks, appends to streaming item:
   > Hello [ref: abc123]
   W
   Wo
   Wel
   Welc

5. LlmActor sends stream_end to llm:0:stream
   LlmTestActor detects it, sends StreamReference:
   { payload: {}, meta: { type: "StreamReference", reference: "abc123" } }

6. Hub forwards StreamReference to ui:user
   TUI receives it, matches reference "abc123", finalizes:
   pending_input → log_items (as confirmed entry)

7. Display:
     [echo result]
     [streaming text...]
   > Hello [ref: abc123]    ← now a normal log item
```

## Loop Problems

### Problem 1: Hub publishes to ui:user on input
When hub receives input on `ui:input`, it was publishing to `ui:user` with `meta.type = "UserInput"`. The TUI subscribes to `ui:user`, so it receives its own message back.

**Fix:** Hub should NOT publish to `ui:user` for input. The TUI manages `pending_input` locally.

### Problem 2: LlmTestActor publishes back to llm:0:stream
LlmTestActor subscribes to `llm:0:stream`, receives chunks from LlmActor, then publishes them back to `llm:0:stream`. Since it's a broadcast channel, it receives its own messages, creating an infinite loop.

**Fix:** LlmTestActor should skip messages where `source == self.id()`.

### Problem 3: TUI receives stream chunks twice
TUI subscribes to `llm:0:stream` directly AND hub forwards `llm:0:stream` → `ui:user`. So stream chunks arrive via two paths.

**Current workaround:** TUI has two separate receivers (one for `ui:user`, one for `llm:0:stream`). This works but is fragile.

## Proposed Clean Architecture

All messages flow through the hub to `ui:user`. The TUI subscribes to only `ui:user`.

```
TUI
  └─ subscribes to: ui:user
      └─ receives: stream chunks, StreamReference, echo, reverse, etc.

Hub
  └─ subscribes to: llm:0:stream, sar:echo, sar:reverse
      └─ forwards all to: ui:user

LlmTestActor
  ├─ subscribes to: llm:0:stream (read-only, for detecting stream_end)
  ├─ publishes StreamReference to: llm:0:stream (hub forwards to ui:user)
  └─ does NOT forward stream chunks
```

**Pros:**
- Single subscription for TUI (simpler)
- All frontends get all messages uniformly
- Hub is the single source of truth

**Cons:**
- Hub must handle all message type detection
- Stream chunks flow through hub (extra hop)

## StreamReference Matching

The TUI must match `StreamReference` to the correct pending input:

1. When TUI publishes user input to `ui:input`, it generates a UUID reference
2. The reference is embedded in the pending input display: `> <text> [ref: <uuid>]`
3. When `StreamReference` arrives (via `ui:user`), TUI:
   a. Extracts the reference from `meta.reference`
   b. Checks if any pending input ends with `[ref: <uuid>]`
   c. If match: removes the `[ref: ...]` suffix, adds as normal log item, clears pending
   d. If no match: ignore (could be from a different frontend or a stale reference)

## Edge Cases

1. **Multiple frontends:** Each frontend generates its own reference. When one frontend's input is confirmed, only that frontend's pending input is finalized. Other frontends' pending inputs remain.

2. **Stream lag:** If TUI lags behind the stream (broadcast lag), it may miss chunks. The `StreamReference` still arrives and finalizes the pending input. The streaming content will be incomplete but the input is confirmed.

3. **Stream never completes:** If the LLM connection drops, `StreamReference` never arrives. The pending input stays at the bottom indefinitely. Consider a timeout mechanism.

4. **Hub restart:** Hub loses its subscribe-to/route-to state. Needs to re-subscribe on restart.

5. **TUI restart:** TUI loses `pending_input` state. New pending input will be created on next user input.

## Implementation Checklist

- [ ] TUI: remove direct subscription to `llm:0:stream`
- [ ] TUI: add `pending_input: String` field
- [ ] TUI: `render_log()` shows pending input at bottom
- [ ] TUI: on input submit, generate UUID, store in `pending_input`
- [ ] TUI: on `StreamReference` detection (via `ui:user`), match and finalize
- [ ] LlmTestActor: store `last_reference` from input message
- [ ] LlmTestActor: detect `stream_end`, send `StreamReference` confirmation
- [ ] LlmTestActor: skip self-messages on stream topic
- [ ] Hub: do NOT publish to `ui:user` for input messages
- [ ] Tests for pending input lifecycle