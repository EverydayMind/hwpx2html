'use strict';
self.onmessage = async ({data}) => {
  let api, namePointer, nameLength, inputPointer, inputLength, handle;
  try {
    self.postMessage({stage: 'engine'});
    const {instance} = await WebAssembly.instantiate(data.wasm, {});
    api = instance.exports;
    const name = new TextEncoder().encode(data.name);
    nameLength = name.length;
    namePointer = api.hp_alloc(nameLength);
    new Uint8Array(api.memory.buffer, namePointer, nameLength).set(name);
    inputLength = data.input.byteLength;
    inputPointer = api.hp_alloc(inputLength);
    new Uint8Array(api.memory.buffer, inputPointer, inputLength).set(new Uint8Array(data.input));
    data.input = null;
    data.wasm = null;
    self.postMessage({stage: 'convert'});
    // hp_convert owns the input from this point, including the failure path.
    const consumedPointer = inputPointer;
    inputPointer = undefined;
    handle = api.hp_convert(namePointer, nameLength, consumedPointer, inputLength, data.flags);
    const metadata = JSON.parse(new TextDecoder().decode(new Uint8Array(
      api.memory.buffer, api.hp_metadata_pointer(handle), api.hp_metadata_length(handle))));
    // One byte copy out of wasm; transfer to the UI without an HTML string.
    const html = new Uint8Array(api.memory.buffer,
      api.hp_html_pointer(handle), api.hp_html_length(handle)).slice().buffer;
    api.hp_result_free(handle);
    handle = undefined;
    api.hp_free(namePointer, nameLength);
    namePointer = undefined;
    self.postMessage({metadata, html}, [html]);
  } catch (error) {
    self.postMessage({failure: '변환 엔진을 실행하지 못했습니다. 브라우저 메모리와 파일 실행 정책을 확인하거나 CLI를 사용해 주세요.', detail: String(error)});
  } finally {
    // After a wasm trap discard the entire instance rather than touching its
    // possibly interrupted allocator. The parent always terminates this worker.
    self.close();
  }
};
