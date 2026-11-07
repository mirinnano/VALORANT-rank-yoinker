# Desktop design

Scouting and self-review app for VALORANT. It runs beside the game, so it stays light and legible at a glance.

The look follows the RINJIN CLAB site in spirit: ink and paper, heavy italic display type, small mono labels, and halftone
photography with print-style colour offsets. Each screen has one loud moment and quiet data around it.

## Screens

The app draws nothing over the game. Everything lives in the main window, which is meant for a second monitor or Alt-Tab.

| Screen | Job | Opens when |
| --- | --- | --- |
| Home | Rank, last 24 hours, RR trend, recent matches, repeated issues | Menus |
| Match | Rank axis, player table, player detail | Agent select starts |
| Review | Result, findings, round strip, notes, scoreboard | A match ends |
| Stats | Your aggregated metrics, by round type, map and agent | Manual |
| Encounters | Searchable list of past players | Manual |
| Diagnostics | Connection and field checks, anonymized recording | Manual |

## Window

The window has no system title bar. The header is the drag region, double-clicking it toggles maximize, and it carries
minimize, maximize and close buttons.

## Agent select

Agent select has its own screen: five large portraits, the phase clock, team composition, rank spread and map fit.
No public API offers global pick rates, so the app observes every player in your match details and your allies' latest
matches, then stores map, agent, result and tier only. Rates come from your rank band and fall back to all ranks below five
matches. The sample count is always shown.

Agent select reuses the rank axis from the match screen, so ally spread reads the same way in both phases.

Recommendations favour agents you can actually play. Two slots come from your recent pool and the last may come from outside
it, marked as a stretch pick. Scores combine the missing role, the map, your own record and ally synergy. Locked allies count
fully, selecting allies count half, and locked agents are excluded. Role win-rate gaps are small in published data, so role
weights stay modest. Win rates shrink toward 50% with 20 prior matches, and pairs under 5 matches are ignored. The weights in
`ui/pick.js` are initial values that have not been fitted to real data.

## Metrics

Everyone gets ACS, ADR, KAST, first kill and first death rates from their latest competitive match. Your own numbers are
aggregated over the last 20 matches, stored one match per refresh to stay under rate limits. KAST counts a trade when a
teammate kills your killer within 5 seconds. Attack and defense come from the bomb planter, so unplanted matches are left out.

The stats screen adds DDΔ, Econ, first kill difference, clutches, trades, survival, form against older matches, results
after a win or loss, and time of day. Benchmarks differ between sources, so numbers are compared with your own history
instead of being graded. The metrics listed first change with the role you play most.

## Match alert

When the state moves from menus into a match, the app plays a tone, flashes the taskbar if unfocused, and shows a Windows
notification. It never takes focus, and it stays silent when started during a match. Toasts can fail for an uninstalled
portable executable, so the tone and flash carry the alert on their own. The setting lives on the Settings screen.

## Tokens

All colours and motion live in `ui/tokens.css`. Blue is the ally team, red is the enemy team and losses, yellow is you.
Text over images sits on a neutral scrim. Display type is Archivo, labels are JetBrains Mono, and Japanese is Murecho.

## External assets

Images load from `media.valorant-api.com` and fonts from Google Fonts at runtime, so nothing from Riot is bundled.
Only URLs on the media host are accepted, the CSP allows no other image host, and the app works without either.

| Motion | Duration | Use |
| --- | --- | --- |
| `--motion-fast` | 120ms | Press and hover |
| `--motion-base` | 180ms | Round strip, score and status rise |
| `--motion-slow` | 240ms | Axis marks, automatic view change |

`--ease-out` is the single easing curve. Manual navigation, keyboard actions and the three second refresh do not animate.

Personality is calm and exact, with no overshoot. Motion marks the moments that change what the player should read.

| Moment | Sequence |
| --- | --- |
| Match found | Match view rises, the map title wipes in, then ally marks spread from the axis |
| Rank arrives | That row's rank rises once |
| Enemies appear | Enemy marks spread from the axis, 35ms apart |
| Agent locked | Cell flashes and the name rises 3px |
| Score changes | Only the score value rises |
| Match ends | Result wipes in, then figures, round wave and findings, finished in about 650ms |
| Waiting for VALORANT | A dot breathes. This is the only looping motion and it stops once connected |
Only `transform`, `opacity`, `clip-path` and `background-color` animate. Reduced motion keeps fades and drops travel.

## Privacy rules

- Names of Streamer Mode players are never requested. They appear as the agent name.
- Hidden levels are removed before data reaches the UI.
- `puuid` never reaches the UI. Rows use a per-match key.
- Review screens label other players by agent and never read names from match details.

## Diagnostics

The Diagnostics screen runs every API once and checks the fields the app reads. A missing field that the original vRY read is a failure.
A missing field that has not been confirmed on a real client is a warning. Results hold field names and statuses only.

Save a recording to capture the current responses. IDs become stable pseudonyms, names and tokens are removed, and the file
is refused if an original value survives. Replay it on any machine with `VRY_REPLAY=<path>`.

## States

Every screen handles waiting for VALORANT, empty data, and failed loads with a plain message and the next step.
A missing value shows `－`. A hidden level shows `非公開` in the detail pane.

## Keyboard

Up and Down move through the player table. Tab reaches navigation, rounds, tags and notes in reading order.
Focus uses a 2px `--own` outline.
