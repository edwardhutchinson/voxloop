// The client watching its own output path: the half of loop health a server cannot see.
//
// This is part of the client's Audio module (`modules.md`), beside `audio.js` rather than
// inside it, because it is about the one device Audio does not choose — where the sound comes
// out — and it outlives no carriage and builds none.
//
// **The beacon proves audio reached the browser, and says nothing about whether it reached the
// operator's ears** (ADR-0017). The likeliest real *"I can't hear Flight"* is not a routing
// fault at all: it is a headset unplugged, or the operating system quietly repointing its
// default output to the laptop's speakers under a meeting-room projector. Both are invisible
// to the server and invisible to the beacon. So the console checks for itself, three ways:
//
// - **a confirmed check tone at assume** — a tone played, and the operator saying they heard
//   it, which is the only confirmation there is that the path ends at a person;
// - **`devicechange` monitoring thereafter**;
// - **a default-output swap**, detected by comparing the label and group of the `default`
//   entry across change events, because that entry's id is `default` whatever it points at.
//
// Any change is surfaced loudly. *(The swap detection is asserted at moderate confidence and
// is to be confirmed on real hardware, with #17.)*

/**
 * The outputs as `enumerateDevices` has them now, cut to what is compared.
 *
 * `theDefault` is the browser's `default` entry — the label names the device the operating
 * system is sending sound to, and the group is the physical device it belongs to — or `null`
 * where the browser lists none. `outputs` is every real output by id, so that one going away
 * can be told apart from one arriving. Windows' `communications` entry is a second alias
 * rather than a device, like `default`, and is left out for the same reason.
 *
 * @param {{ kind: string, deviceId: string, label: string, groupId: string }[]} devices
 */
export function outputsAsTheyStand(devices) {
	const outputs = devices.filter((device) => device.kind === 'audiooutput');
	const theDefault = outputs.find((device) => device.deviceId === 'default');

	return {
		theDefault: theDefault ? { label: theDefault.label, group: theDefault.groupId } : null,
		outputs: outputs
			.filter((device) => device.deviceId !== 'default' && device.deviceId !== 'communications')
			.map((device) => device.deviceId)
	};
}

/**
 * What moved between two readings of the outputs that an operator has to be told about, or
 * `null` where nothing did.
 *
 * `swapped` is the new default's label, where the default now points at another device.
 * `removed` is an output that was there and is not — a headset unplugged, whether or not it
 * was the default, because the operator may have chosen it. **An output arriving is not
 * news**: nothing that was reaching somebody has stopped.
 *
 * **Nothing is compared across a reading with no labels.** A browser that has not been granted
 * a device hands its entries back blank, and a swap reported off two blank labels would be an
 * alarm about nothing — the one kind that teaches an operator to dismiss the next one.
 */
export function whatMoved(before, after) {
	if (before.theDefault?.label === '' || after.theDefault?.label === '') return null;

	const moved = {};
	if (
		before.theDefault &&
		after.theDefault &&
		(before.theDefault.label !== after.theDefault.label ||
			before.theDefault.group !== after.theDefault.group)
	) {
		moved.swapped = after.theDefault.label;
	}
	if (before.outputs.some((output) => !after.outputs.includes(output))) moved.removed = true;

	return Object.keys(moved).length === 0 ? null : moved;
}

/**
 * Watch the outputs for as long as a session lasts, and say when one moves.
 *
 * Answers with `settle`, which takes the outputs as they stand now as the ones to compare
 * against — called when the operator confirms the check tone, because what they just heard it
 * on is the path they are relying on — and `stop`.
 *
 * `devices` is `navigator.mediaDevices`, handed in so that nothing here reaches for a global.
 */
export function watchTheOutput({ devices, onMoved }) {
	let before = null;
	let stopped = false;

	const read = async () => outputsAsTheyStand(await devices.enumerateDevices());

	async function settle() {
		before = await read();
	}

	async function changed() {
		const now = await read();
		const moved = before && whatMoved(before, now);
		before = now;
		if (moved && !stopped) onMoved(moved);
	}

	settle();
	devices.addEventListener('devicechange', changed);

	return {
		settle,
		stop() {
			stopped = true;
			devices.removeEventListener('devicechange', changed);
		}
	};
}

/**
 * Play the check tone: a short, plain tone through whatever the browser plays sound on.
 *
 * It goes out on the same default output the loops play through, so hearing it is hearing the
 * path the loops take. It answers when the tone has finished, so that the question *did you
 * hear it?* is asked after there was something to hear.
 */
export async function playTheCheckTone() {
	const context = new AudioContext();
	const tone = context.createOscillator();
	const level = context.createGain();
	tone.frequency.value = 660;
	// Well under full scale: a check is for whether sound arrives, not how loud it can be.
	level.gain.value = 0.2;
	tone.connect(level).connect(context.destination);

	const finished = new Promise((resolve) => (tone.onended = resolve));
	tone.start();
	tone.stop(context.currentTime + 0.6);
	await finished;
	await context.close();
}
