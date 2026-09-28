# Game controls

Game mode translates desktop input into touchscreen contacts. It does not use
the game's internal APIs or inspect its state. HUD positions and the game's
own hold/toggle settings determine what each press does.

```sh
iphone-mirror-rs --connection wifi --game-profile ~/.config/iphone-mirror-rs/rainbow-six.profile
```

The normal mirror controls stay active until you press **F8**. A new profile
needs calibration first. Use the game's training area or HUD editor where its
controls are visible; menu buttons are not the gameplay HUD.

## Calibrate

Press **F9** and click each target requested by the status strip above Home:
joystick center, an empty area for looking, lean left, lean right, reload,
interact, melee, grenade, primary weapon, secondary weapon, crouch, fire, aim.
These calibration clicks stay in the viewer; they do not tap the phone.
Escape cancels. The final click saves the profile atomically. F9 repeats the
calibration; different layouts can use different profile files.

To map one key without repeating setup, press **F10**, press the desired key,
then click its on-screen control. Use **Space** for vault/climb, **X** for rappel,
and **B** for a second throwable or gadget. Existing profiles can still enter
game mode; these optional actions remain inactive until calibrated. An unmapped
action logs a calibration warning and sends no guessed touch. F10 calibration
clicks stay local, save immediately, and preserve every other binding. Escape
cancels without saving. Contextual actions must be visible at an eligible ledge
or wall, or exposed in the game's HUD editor.

For sprint, use **F10 → Shift → click the forward sprint endpoint above the
joystick**. Its vertical distance replaces the default sprint multiplier;
the joystick center remains unchanged. Choose the point the movement finger
must reach, rather than a separate action button. Test in the training area
with ADS/crouch off. This calibration changes the drag endpoint, not the game's
rules, and does not intentionally release on the sprint-lock icon.

Coordinates are relative to the displayed phone image, excluding margins and
the Home strip. Resize the window freely. Recalibrate after moving game HUD
buttons. Rotation or a change of video dimensions exits game mode; re-enter
only when the corresponding HUD is displayed.

Some controls are contextual or hidden by the game's settings. Expose them in
the HUD editor before calibration, or edit their coordinates in the profile.
Mapping two keys to the same weapon-toggle button will toggle for both keys;
direct primary/secondary selection requires two distinct on-screen targets.

For Rainbow Six Mobile's Pro layout, enable the **Melee** button in the HUD
editor and set both **Double Tap Effect** settings to **No Double Tap Action**.
Otherwise repeated joystick presses or look recentering can trigger ping/melee
shortcuts. The viewer does not change these game settings automatically.

Enable **Lock Joystick in Place** so its origin matches your calibration.
For manual firing, set **Gameplay → Input → ADS Auto-Shoot Behavior** and
**Hip Fire Auto-Shoot Behavior** to **Off**. Hip-fire auto-shoot can also enter
ADS when a target is out of range; that behavior comes from the game.
For leaning without ADS, enable the game's **Hip Lean** option in Gameplay
Settings, described in [Ubisoft's Hip Lean release notes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2388-patch-notes---1-5-toxic-fog/?p=web).
The viewer sends the calibrated lean touch; it does not separately press ADS
when Q/E are used. Recalibrate if changing this option changes HUD positions.

## Play

Focus the mirror and press **F8**. The cursor locks and hides; **Escape** or F8
releases every contact and restores it. Focus loss also exits. F1 releases game
controls before sending Home. The viewer refuses capture if pointer locking is
unavailable, rather than letting aiming stop at the desktop edge.
Exiting discards queued game gestures and releases the contacts already sent
to the phone, so pending look recenters do not replay after Escape.

| Input | Touch target |
| --- | --- |
| W / A / S / D | Movement joystick; diagonals have the same radius |
| Hold Shift + W (optionally A / D) | Extend forward joystick displacement for sprint |
| Q / E | Lean left / right |
| R | Reload |
| F | Interact |
| V | Melee |
| G | Grenade |
| B | Second throwable / gadget (requires its own calibration) |
| Space | Vault / climb (contextual; requires calibration) |
| X | Rappel (contextual; requires calibration) |
| 1 / 2 | Primary / secondary weapon |
| C | Crouch |
| Left / right mouse button | Fire / aim down sights |
| Mouse motion | Drag in the calibrated looking area |

A bound button stays touched until its key/button is released. Choose hold or
toggle behavior in the game's settings. Unbound keys and the scroll wheel are
suppressed while captured, so movement keys cannot accidentally type into a
text field. Exit game mode to use menus or normal keyboard input.

Releasing WASD immediately centers the joystick, then lifts its touch after
150 ms with no movement keys held. A direction change within that grace period
reuses the origin; a longer pause starts a fresh gesture. This prevents idle
movement touches from persisting indefinitely across gameplay screen changes.
The viewer does not detect death/respawn; if controls still become stuck, use
Escape then F8 to reset all contacts. Escape, focus loss and leaving game mode
always lift contacts immediately.
Either Shift key works; sprint stays requested until both are released. Shift
alone does not start movement. Releasing Shift restores normal displacement
without lifting the movement contact or disturbing look/lean/fire contacts.
The game decides whether the larger displacement activates sprint, including
its restrictions while aiming, crouching or moving diagonally.

## Tuning and limits

Profiles are small `name=value` text files with `version=1`. Targets use
`name=x,y`, with both coordinates between 0 and 1. Calibration fills them in.
`sensitivity=0.002` is the fraction of the phone's displayed short edge per
raw relative mouse unit (device/backend dependent, not screen pixels). Lower it
for slower aiming. `joystick_radius=0.08` is
also measured relative to the short edge; increase it if movement does not
reach full speed. `sprint_multiplier=2` multiplies that radius only while Shift
and net forward movement are held. Values 1..4 are accepted; the resulting
radius is capped at 0.5 of the short edge and coordinates stay within the image.
Existing profiles default to 2 without recalibration. Tune it to the game's
sprint threshold; this is a held joystick gesture, not a separate sprint-button
tap or an intentional sprint-lock release. Ubisoft describes sprint-lock as
[dragging past the joystick circle and releasing on the sprint icon](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2266-how-do-i-automatically-run-sprint-lock/?p=web).
Restart the viewer after editing the file.
The optional `sprint=x,y` endpoint takes priority over `sprint_multiplier`;
its y coordinate must be above `joystick`, and its x coordinate is not used
to steer. Optional `vault`, `rappel`, and `secondary_gadget` fields use ordinary
target coordinates. A key targets a HUD slot, not a named item: B may activate
a different gadget on another operator. Separate profile files are appropriate
when the layout changes. F10 saves and applies a mapping without a restart.

The protocol supports **five contacts**. Movement and looking each reserve one;
three action buttons can be held together. Additional action presses are
rejected until a button is released and pressed again. Key repeat never starts
another touch.

Looking lifts and recenters its own contact at the edge of a small region while
preserving movement and buttons. Mouse vectors are split at region boundaries
and continued from the center in the same callback, preserving displacement
within contact-coordinate rounding. Work is limited to eight segment attempts
per callback; extreme deltas or outward motion from an edge calibration are
discarded beyond that bound and counted as `clipped_mouse_events`. No remainder
is queued for replay after the mouse stops. `look_resets` counts reanchors.
This is touch-based aiming: sensitivity, acceleration,
touch sampling and game behavior still apply. It is not guaranteed to behave
like native PC raw-mouse input.

A new joystick drag holds its center for 20 ms before moving. Back-to-back
down/move reports were merged on the tested phone, losing the initial drag.
Ordinary direction changes, looking and button presses add no such delay,
although they can wait behind the initial joystick press in the ordered input
queue. The joystick pays this setup cost on first movement and when movement
resumes after the idle contact has been lifted.

## Live qualification

Tested over Wi-Fi with an iPhone 15 / iOS 27 in Rainbow Six Mobile's shooting
range: concurrent movement/look/fire, primary/secondary selection, reload,
both lean directions, crouch, melee, grenade preparation and Escape while
controls were held. A five-contact combination of movement, look, fire, aim
and lean was exercised. These checks used an isolated Sway desktop; physical
mouse sensitivity remains a personal tuning setting. The contextual interact
mapping has not been qualified against an eligible interaction in that range.
The Safari diagnostic below has not been live-qualified on this phone.

Direction-change debugging reproduced lost initial joystick displacement with
back-to-back reports and verified movement after adding the initial 20 ms
spacing. Short WASD handoffs with look/lean held, neutral stopping, manual fire
and Escape were checked in the range. With both auto-shoot options off, hovering
over a training target did not fire or enter ADS. These are functional checks,
not a measurement of end-to-end input latency or a guarantee of PC-like aiming.

Shift sprint has offline coverage for both Shift keys, release/exit cleanup,
diagonals, rotations and preservation of simultaneous contacts. Its default
radius did not reliably activate sprint in the user's live match. The explicit
endpoint calibration and new vault/rappel/second-gadget bindings have offline
coverage but still require live HUD calibration and gameplay verification.
Idle contact expiration passes timer and quick-handoff regression tests, but
the reported death/respawn issue still needs a live retest with this change.

## Diagnostic page

```sh
cargo run --example touch_page -- --bind 0.0.0.0 --port 46568
```

Open the printed port at this computer's LAN address in Safari on the phone.
The temporary page displays active/max contacts and logs synthetic touch
snapshots. Calibrate a **separate test profile** to that page. Check movement and
looking simultaneously, add buttons, release one while the others remain held,
then Escape: active contacts must return to zero. Stop the server with Ctrl+C.
Its endpoint accepts diagnostic events without authentication; use it only on
a trusted local network. Do not publish private gameplay screenshots or profiles
as test fixtures.
