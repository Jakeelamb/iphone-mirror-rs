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
| Q / E | Lean left / right |
| R | Reload |
| F | Interact |
| V | Melee |
| G | Grenade |
| 1 / 2 | Primary / secondary weapon |
| C | Crouch |
| Left / right mouse button | Fire / aim down sights |
| Mouse motion | Drag in the calibrated looking area |

A bound button stays touched until its key/button is released. Choose hold or
toggle behavior in the game's settings. Unbound keys and the scroll wheel are
suppressed while captured, so movement keys cannot accidentally type into a
text field. Exit game mode to use menus or normal keyboard input.

After the first movement press, the joystick contact stays down at its center
when you release WASD. This stops movement without restarting the touch during
quick direction changes. Escape, focus loss and leaving game mode lift it.

## Tuning and limits

Profiles are small `name=value` text files with `version=1`. Targets use
`name=x,y`, with both coordinates between 0 and 1. Calibration fills them in.
`sensitivity=0.002` is the fraction of the phone's displayed short edge per
relative mouse pixel. Lower it for slower aiming. `joystick_radius=0.08` is
also measured relative to the short edge; increase it if movement does not
reach full speed. Restart the viewer after editing the file.

The protocol supports **five contacts**. Movement and looking each reserve one;
three action buttons can be held together. Additional action presses are
rejected until a button is released and pressed again. Key repeat never starts
another touch.

Looking lifts and recenters its own contact at the edge of a small region while
preserving movement and buttons. An unusually large mouse delta is clipped at
that region's edge. This is touch-based aiming: sensitivity, acceleration,
touch sampling and game behavior still apply. It is not guaranteed to behave
like native PC raw-mouse input.

A new joystick drag holds its center for 20 ms before moving. Back-to-back
down/move reports were merged on the tested phone, losing the initial drag.
Ordinary direction changes, looking and button presses add no such delay,
although they can wait behind the initial joystick press in the ordered input
queue. The joystick pays this setup cost once per game-mode entry.

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
