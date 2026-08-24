export class FramebufferPresenter {
  #context;
  #memory;
  #renderer;
  #view = null;
  #viewBuffer = null;
  #viewPointer = -1;
  #viewLength = -1;
  #viewRebuilds = 0;

  constructor(context, memory, renderer) {
    this.#context = context;
    this.#memory = memory;
    this.#renderer = renderer;
  }

  present() {
    let wasmBoundaryCalls = 0;
    const width = this.#renderer.width();
    wasmBoundaryCalls += 1;
    const height = this.#renderer.height();
    wasmBoundaryCalls += 1;
    const pointer = this.#renderer.framebuffer_ptr();
    wasmBoundaryCalls += 1;
    const length = this.#renderer.framebuffer_len();
    wasmBoundaryCalls += 1;
    const expectedLength = width * height * 4;
    if (length !== expectedLength) {
      throw new Error(`프레임버퍼 길이 불일치: expected=${expectedLength}, actual=${length}`);
    }

    const memoryBuffer = this.#memory.buffer;
    if (
      this.#view === null ||
      this.#viewBuffer !== memoryBuffer ||
      this.#viewPointer !== pointer ||
      this.#viewLength !== length
    ) {
      this.#view = new Uint8ClampedArray(memoryBuffer, pointer, length);
      this.#viewBuffer = memoryBuffer;
      this.#viewPointer = pointer;
      this.#viewLength = length;
      this.#viewRebuilds += 1;
    }

    this.#context.putImageData(new ImageData(this.#view, width, height), 0, 0);
    return wasmBoundaryCalls;
  }

  get viewRebuilds() {
    return this.#viewRebuilds;
  }
}
