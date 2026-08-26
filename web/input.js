export const INPUT_FORWARD = 1 << 0;
export const INPUT_BACKWARD = 1 << 1;
export const INPUT_LEFT = 1 << 2;
export const INPUT_RIGHT = 1 << 3;
export const INPUT_UP = 1 << 4;
export const INPUT_DOWN = 1 << 5;

const INPUT_FLAG_DRAGGING = 1 << 0;
const INPUT_MODIFIER_SHIFT = 1 << 1;
const INPUT_MODIFIER_CONTROL = 1 << 2;
const INPUT_MODIFIER_ALT = 1 << 3;
const INPUT_MODIFIER_META = 1 << 4;

const CODE_BITS = new Map([
  ["KeyW", INPUT_FORWARD],
  ["ArrowUp", INPUT_FORWARD],
  ["KeyS", INPUT_BACKWARD],
  ["ArrowDown", INPUT_BACKWARD],
  ["KeyA", INPUT_LEFT],
  ["ArrowLeft", INPUT_LEFT],
  ["KeyD", INPUT_RIGHT],
  ["ArrowRight", INPUT_RIGHT],
  ["KeyE", INPUT_UP],
  ["Space", INPUT_UP],
  ["KeyQ", INPUT_DOWN],
  ["ShiftLeft", INPUT_DOWN],
  ["ShiftRight", INPUT_DOWN],
]);

const KEY_BITS = new Map([
  ["w", INPUT_FORWARD],
  ["arrowup", INPUT_FORWARD],
  ["s", INPUT_BACKWARD],
  ["arrowdown", INPUT_BACKWARD],
  ["a", INPUT_LEFT],
  ["arrowleft", INPUT_LEFT],
  ["d", INPUT_RIGHT],
  ["arrowright", INPUT_RIGHT],
  ["e", INPUT_UP],
  [" ", INPUT_UP],
  ["q", INPUT_DOWN],
  ["shift", INPUT_DOWN],
]);

function keyBinding(event) {
  const codeBit = CODE_BITS.get(event.code);
  if (codeBit !== undefined) {
    return { id: `code:${event.code}`, bit: codeBit };
  }
  const key = String(event.key).toLowerCase();
  const keyBit = KEY_BITS.get(key);
  return keyBit === undefined ? null : { id: `key:${key}`, bit: keyBit };
}

function modifierFlags(event) {
  return (
    (event.shiftKey ? INPUT_MODIFIER_SHIFT : 0) |
    (event.ctrlKey ? INPUT_MODIFIER_CONTROL : 0) |
    (event.altKey ? INPUT_MODIFIER_ALT : 0) |
    (event.metaKey ? INPUT_MODIFIER_META : 0)
  );
}

export class InputCollector {
  #canvas;
  #document;
  #held = new Map();
  #pressedBits = 0;
  #releasedBits = 0;
  #pointerDx = 0;
  #pointerDy = 0;
  #wheelDelta = 0;
  #pointerButtons = 0;
  #modifierFlags = 0;
  #draggingPointerId = null;
  #pendingDrag = false;
  #lastPointer = null;
  #listeners = [];

  constructor(canvas, windowObject = window, documentObject = document) {
    this.#canvas = canvas;
    this.#document = documentObject;

    this.#listen(windowObject, "keydown", (event) => this.#onKeyDown(event));
    this.#listen(windowObject, "keyup", (event) => this.#onKeyUp(event));
    this.#listen(canvas, "pointerdown", (event) => this.#onPointerDown(event));
    this.#listen(canvas, "pointermove", (event) => this.#onPointerMove(event));
    this.#listen(canvas, "pointerup", (event) => this.#finishPointer(event, true));
    this.#listen(canvas, "pointercancel", (event) => this.#finishPointer(event, false));
    this.#listen(canvas, "lostpointercapture", (event) =>
      this.#finishPointer(event, false, false),
    );
    this.#listen(
      canvas,
      "wheel",
      (event) => {
        if (this.#isActive()) {
          this.#wheelDelta += event.deltaY;
          this.#modifierFlags = modifierFlags(event);
          event.preventDefault();
        }
      },
      { passive: false },
    );
    this.#listen(windowObject, "blur", () => this.resetHeld());
    this.#listen(documentObject, "visibilitychange", () => {
      if (documentObject.hidden) {
        this.resetHeld();
      }
    });
  }

  #listen(target, type, listener, options) {
    target.addEventListener(type, listener, options);
    this.#listeners.push({ target, type, listener, options });
  }

  #isActive() {
    return this.#document.activeElement === this.#canvas || this.#draggingPointerId !== null;
  }

  #heldBits() {
    let bits = 0;
    for (const bit of this.#held.values()) {
      bits |= bit;
    }
    return bits;
  }

  #onKeyDown(event) {
    if (!this.#isActive()) {
      return;
    }
    this.#modifierFlags = modifierFlags(event);
    const binding = keyBinding(event);
    if (binding === null) {
      return;
    }
    const before = this.#heldBits();
    if (!this.#held.has(binding.id)) {
      this.#held.set(binding.id, binding.bit);
      this.#pressedBits |= this.#heldBits() & ~before;
    }
    event.preventDefault();
  }

  #onKeyUp(event) {
    this.#modifierFlags = modifierFlags(event);
    const binding = keyBinding(event);
    if (binding === null) {
      return;
    }
    const before = this.#heldBits();
    if (this.#held.delete(binding.id)) {
      this.#releasedBits |= before & ~this.#heldBits();
    }
    if (this.#isActive()) {
      event.preventDefault();
    }
  }

  #onPointerDown(event) {
    if (event.button !== 0 || this.#draggingPointerId !== null) {
      return;
    }
    this.#canvas.focus({ preventScroll: true });
    this.#draggingPointerId = event.pointerId;
    this.#lastPointer = [event.clientX, event.clientY];
    this.#pointerButtons = event.buttons & 0x1f;
    this.#modifierFlags = modifierFlags(event);
    this.#canvas.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  #onPointerMove(event) {
    if (event.pointerId !== this.#draggingPointerId || this.#lastPointer === null) {
      return;
    }
    this.#pointerDx += event.clientX - this.#lastPointer[0];
    this.#pointerDy += event.clientY - this.#lastPointer[1];
    this.#lastPointer = [event.clientX, event.clientY];
    this.#pointerButtons = event.buttons & 0x1f;
    this.#modifierFlags = modifierFlags(event);
  }

  #finishPointer(event, keepPendingDrag, releaseCapture = true) {
    if (event.pointerId !== this.#draggingPointerId) {
      return;
    }
    this.#draggingPointerId = null;
    this.#lastPointer = null;
    if (releaseCapture && this.#canvas.hasPointerCapture(event.pointerId)) {
      this.#canvas.releasePointerCapture(event.pointerId);
    }
    if (keepPendingDrag && (this.#pointerDx !== 0 || this.#pointerDy !== 0)) {
      this.#pendingDrag = true;
    } else if (!keepPendingDrag) {
      this.#pointerDx = 0;
      this.#pointerDy = 0;
    }
    this.#pointerButtons = event.buttons & 0x1f;
    this.#modifierFlags = modifierFlags(event);
  }

  snapshot() {
    const values = new Float64Array([
      this.#heldBits(),
      this.#pressedBits,
      this.#releasedBits,
      this.#pointerDx,
      this.#pointerDy,
      this.#wheelDelta,
      this.#pointerButtons,
      this.#modifierFlags |
        (this.#draggingPointerId === null && !this.#pendingDrag ? 0 : INPUT_FLAG_DRAGGING),
    ]);
    this.#pressedBits = 0;
    this.#releasedBits = 0;
    this.#pointerDx = 0;
    this.#pointerDy = 0;
    this.#wheelDelta = 0;
    this.#pendingDrag = false;
    return values;
  }

  resetHeld() {
    this.#releasedBits |= this.#heldBits();
    this.#held.clear();
    const pointerId = this.#draggingPointerId;
    this.#draggingPointerId = null;
    this.#pendingDrag = false;
    this.#lastPointer = null;
    if (pointerId !== null && this.#canvas.hasPointerCapture(pointerId)) {
      try {
        this.#canvas.releasePointerCapture(pointerId);
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "NotFoundError")) {
          throw error;
        }
      }
    }
    this.#pointerDx = 0;
    this.#pointerDy = 0;
    this.#wheelDelta = 0;
    this.#pointerButtons = 0;
    this.#modifierFlags = 0;
  }

  debugState() {
    return {
      heldBits: this.#heldBits(),
      dragging: this.#draggingPointerId !== null,
      pointerButtons: this.#pointerButtons,
    };
  }

  dispose() {
    for (const { target, type, listener, options } of this.#listeners) {
      target.removeEventListener(type, listener, options);
    }
    this.#listeners = [];
    this.resetHeld();
  }
}
