# Conversation Timeline Design

## Goal

Add a lightweight timeline to the conversation page so users can see each question they asked and jump back to it quickly.

## User experience

A fixed vertical timeline appears near the right edge of the active conversation page. Each user question is represented by one circular marker. Markers are light gray by default and become darker on hover or when active.

Hovering a marker opens a dark tooltip to the left of the marker. The tooltip shows a single-line question summary capped at 40 characters, with overflow replaced by an ellipsis.

Clicking a marker smoothly scrolls the conversation to the corresponding user message and briefly highlights that message so the jump target is clear.

## Architecture

Implement the feature in `codex_session_delete/inject/renderer-inject.js` as renderer-side DOM injection, matching the existing Codex++ enhancement pattern.

The feature does not add a backend API and does not modify the Codex app bundle. It injects:

- timeline CSS through the existing style injection path
- one fixed timeline container in the conversation page
- one marker per detected user question
- tooltip and active/highlight behavior through DOM event listeners

## Data source

The injected script scans the current conversation DOM for user message nodes. For each user message it records:

- the message DOM node used as the scroll target
- plain text content used for the tooltip summary
- the message position used to place the timeline marker

Detection should prefer stable role/message structure inside the Codex conversation content. If no explicit role marker is available, matching should stay inside the main conversation region and conservatively match user-styled message nodes to avoid collecting sidebar, menu, or settings text.

Empty or whitespace-only user messages are skipped.

## Update flow

A `MutationObserver` watches the conversation area and rebuilds the timeline when messages change, the user sends a new question, or the active conversation rerenders. Rebuilds must remove previous timeline markers before inserting new ones so duplicate markers are not created.

If no user questions are detected, the timeline container is hidden or removed. If one question is detected, the timeline still shows one marker.

## Marker positioning

Markers are distributed along the fixed right-side vertical track based on each message's relative position in the current conversation scrollable area. This keeps markers aligned with the rough location of each question without requiring complex virtualization support.

When markers are close together they may visually cluster; the first version will not add grouping or aggregation.

## Edge cases

- No user questions: do not show the timeline.
- One user question: show one marker.
- Empty user message: skip it.
- Long user question: truncate only the tooltip summary, not the source text or scroll target.
- Conversation rerender: rebuild without duplicating containers or markers.
- Missing conversation container: do nothing and retry on later mutations.

## Testing

Automated tests should cover the pure logic where practical:

- 40-character summary truncation with ellipsis
- empty/whitespace questions are skipped
- timeline rebuild removes old markers before inserting new ones
- marker click calls scroll behavior and applies a temporary highlight class to the target message

Manual UI verification is required after implementation:

1. Open a conversation with multiple user questions.
2. Confirm the right-side vertical timeline appears.
3. Hover each marker and confirm the tooltip shows the expected question summary.
4. Click a marker and confirm the conversation scrolls to the matching user message.
5. Send a new question and confirm a new marker appears without duplicating old markers.
