# Event Loop

## Overview

Each datatype instance owns a dedicated `EventLoop`. It runs on a `spawn_blocking` thread and is responsible for:
- Triggering push/pull sync after local writes complete
- Reacting to server-side realtime notifications
- Managing exponential backoff on transient errors

```
┌─────────────────────────────────────────────────┐
│  TransactionalDatatype / Public API             │
│  (counter.increase(), counter.sync())           │
└──────────────────┬──────────────────────────────┘
                   │ send Event
        ┌──────────▼──────────┐
        │     EventLoop       │  ← spawn_blocking thread
        │  (run loop)         │
        └──────────┬──────────┘
                   │ push_pull()
        ┌──────────▼──────────┐
        │   WiredDatatype     │
        └──────────┬──────────┘
                   │ push_pull()
        ┌──────────▼──────────┐
        │   Connectivity      │  ← LocalConnectivity / NullConnectivity / ...
        └─────────────────────┘
```

---

## Channel Architecture

The event loop receives events through two crossbeam channels.

```
                  ┌─────────────────┐
                  │   EventLoop     │
                  │                 │
  unbounded_tx ──►│  unbounded_rx   │  capacity: unlimited
                  │                 │
    bounded_tx ──►│    bounded_rx   │  capacity: 1
                  └─────────────────┘
```

| Channel | Purpose | Behavior |
|---------|---------|----------|
| `unbounded` | `Stop`, explicit `sync()`, `Notify` | Always polled; read even during BackOff |
| `bounded` | Realtime auto-push, Notify-triggered push | Capacity=1; silently dropped if already queued |

The `unbounded_tx` registered via `connectivity.register(wired, unbounded_tx)` is the handle the server uses to send `Notify` events to this client.

---

## Event Types

```rust
pub enum Event {
    Stop(Sender<()>),                              // Shut down the event loop (includes ack channel)
    PushTransaction(Option<oneshot::Sender<...>>), // Request push/pull (response channel optional)
    BackOff,                                       // BackOff timer expired (internal signal)
    Notify(Notification),                          // Realtime notification from the server
}
```

`PushTransaction` response channel (`resp_tx`):
- `Some(tx)` — sent by `sync()`; blocks caller until complete, returns error if any
- `None` — sent by realtime auto-push or Notify-triggered push; result is discarded

---

## EventLoopAction States

The event loop tracks its current sync policy via `EventLoopAction`.

```
            sync succeeds
  ┌──────────────────────────────────────────┐
  │                                          │
  ▼       error: BackOff              error: PauseSync
Normal ──────────────────► BackOff      ──► PauseSync
  ▲                          │
  │   BackOff timer expires  │
  └──────────────────────────┘
  ▲        or explicit sync() succeeds
  └──────────────────────────────────────────┘
```

| State | Behavior |
|-------|----------|
| `Normal` | Auto-push allowed; both channels polled |
| `BackOff` | Auto-push blocked; only `unbounded_rx` polled; retries after timeout |
| `PauseSync` | Auto-push blocked; any `PushTransaction` immediately returns an error |

---

## receive_event Logic

`receive_event` is called at the start of every loop iteration.

```
receive_event()
  │
  ├── [Normal] push_if_needed && wired.push_if_needed()
  │     → return Ok(Event::PushTransaction(None))   ← auto-push trigger
  │
  ├── [BackOff] compute next backoff duration
  │     crossbeam select! {
  │       recv(unbounded_rx) → handle immediately (Stop, explicit sync, Notify)
  │       default(duration)  → timer expired → return Ok(Event::BackOff)
  │     }
  │
  └── [Normal / PauseSync] no timeout
        crossbeam select! {
          recv(unbounded_rx) → handle
          recv(bounded_rx)   → handle
        }
```

`push_if_needed()` returns `true` only when `is_realtime() && need_push()`. This prevents auto-push from firing in manual sync mode.

---

## Event Handling

### PushTransaction

```
PushTransaction(resp_tx)
  │
  ├── [PauseSync] → immediately return error → process_blocking_resp()
  │
  └── wired.push_pull()
        ├── Ok  → event_loop_action = Normal
        │         backoff = None (cleared if not BackOff)
        │         process_blocking_resp(None)
        │
        └── Err(DatatypeErrorWithActions)
              ├── event_loop_action = dewa.event_loop_action
              ├── wired.handle_error(error, datatype_action)
              └── process_blocking_resp(Some(error))
```

### Notify

```
Notify(notification)
  │
  └── wired.handle_notification(notification) → bool
        ├── false: self-notification (trace) or mismatched duid (warn) → skip
        ├── false: cp_sseq >= notify.sseq → already up-to-date → skip
        │
        └── true: cp_sseq < notify.sseq → push-pull needed
                bounded_tx.try_send(PushTransaction(None))
                  ├── Ok  → processed in next iteration as PushTransaction
                  └── Err → already queued → drop (best-effort)
```

> **Notify during BackOff**: `bounded_rx` is not polled during BackOff. A PushTransaction queued via `bounded_tx` will only be processed after the BackOff timer expires. This is intentional — Notify must not bypass BackOff protection. Explicit `sync()` uses `unbounded_tx` and bypasses BackOff immediately.

### Stop

```
Stop(ack_tx)
  └── ack_tx.send(()) → event loop exits
```

---

## BackOff Details

Exponential backoff is implemented via `backon::ExponentialBuilder`.

| Parameter | Value |
|-----------|-------|
| Minimum delay | 500ms |
| Maximum delay | 30s |
| Max retry count | Unlimited |
| Growth factor | Exponential (×2) |

**BackOff entry**: Transient errors such as `ClientPushPullError::FailedInConnectivity` map to `EventLoopAction::BackOff` via `.mapping()`.

**BackOff exit**:
- Explicit `sync()` succeeds (via unbounded channel, bypasses wait)
- BackOff timer expires → automatic retry succeeds

---

## DatatypeAction — Post-Error State Transitions

On push_pull failure, `DatatypeErrorWithActions.datatype_action` determines the datatype state change.

| Action | Effect |
|--------|--------|
| `Normal` | No state change |
| `Restart` | Transition to `DueToSubscribeOrCreate` (reconnect attempt) |
| `Disable` | Transition to `Disabled` (sync permanently stopped) |
| `Reset` | Call `do_rollback()`, then recover from server snapshot |

---

## Realtime Notification Flow (LocalConnectivity)

In realtime mode, when one client pushes transactions the server immediately notifies all other clients subscribed to the same datatype.

```
Client A                LocalDatatypeServer         Client B
   │                          │                        │
   │── push_pull() ──────►│                        │
   │                          │ notify_pushed()        │
   │                          │── Notify ─────────────►│ (via unbounded_tx)
   │◄── PushPullPack ─────────│                        │
   │                          │                EventLoop::run()
   │                          │                  handle_notification()
   │                          │                    cp_sseq < notify.sseq?
   │                          │                  bounded_tx.try_send(PushTransaction)
   │                          │                        │
   │                          │◄── push_pull() ────│
   │                          │─── PushPullPack ──────►│
```

`handle_notification` filtering logic:
1. `identical_cuid` — notification originated from this client → skip (`trace`)
2. `different_duid` — notification for a different datatype misrouted → skip (`warn`)
3. `cp_sseq >= notify.sseq` — already at or ahead of the notified sseq → skip (`trace`)
4. Otherwise → schedule `PushTransaction` via `bounded_tx`

---

## Event Sender API

| Method | Channel | Blocking | Use case |
|--------|---------|----------|----------|
| `send_push_transaction_with_best_effort()` | bounded | No | Realtime auto-push after local write |
| `send_push_transaction_with_guarantee()` | unbounded | Yes (oneshot wait) | `sync()` call |
| `send_stop()` | unbounded | Yes (ack wait) | Datatype shutdown |

`send_push_transaction_with_best_effort` returns immediately if `is_realtime()` is `false`, preventing accidental auto-push in manual mode.
