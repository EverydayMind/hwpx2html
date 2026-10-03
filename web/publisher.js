'use strict';
(() => {
  const $ = id => document.getElementById(id);
  let file = null, result = null, previewUrl = null, worker = null, workerUrl = null;
  let timer = null, busy = false, generation = 0, reading = false;
  const status = (message, error = false, detail = '') => {
    $('status').textContent = message;
    $('status').classList.toggle('error', error);
    $('error-details').hidden = !detail;
    $('error-detail').textContent = detail;
  };
  const step = name => {
    for (const id of ['select', 'preview', 'publish']) {
      if (id === name) $('step-' + id).setAttribute('aria-current', 'step');
      else $('step-' + id).removeAttribute('aria-current');
    }
  };
  const setBusy = value => {
    busy = value;
    $('settings').disabled = value;
    $('file').disabled = value;
    $('convert').disabled = value || reading || !file;
    $('cancel').hidden = !value;
    $('preview').setAttribute('aria-busy', String(value));
  };
  const stopWorker = () => {
    if (worker) worker.terminate();
    worker = null;
    if (workerUrl) URL.revokeObjectURL(workerUrl);
    workerUrl = null;
    if (timer) clearInterval(timer);
    timer = null;
    setBusy(false);
  };
  const clearResult = () => {
    result = null;
    $('preview').removeAttribute('src');
    $('preview').hidden = true;
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = null;
    $('placeholder').hidden = false;
    $('result-name').textContent = '';
    $('metrics').hidden = true;
    $('metrics').replaceChildren();
    $('warning-card').hidden = true;
    $('warnings').replaceChildren();
    $('ack').checked = false;
    $('publish-card').hidden = true;
    $('publish-controls').disabled = true;
    $('iframe-code').value = '';
    $('div-code').value = '';
    $('manual-copy').hidden = true;
    $('copy-fallback').value = '';
    step('select');
  };
  async function choose(candidate) {
    const ownGeneration = ++generation;
    stopWorker();
    clearResult();
    file = null;
    reading = true;
    $('convert').disabled = true;
    $('file-info').textContent = candidate ? candidate.name + ' · ' + (candidate.size / 1024 / 1024).toFixed(2) + ' MiB' : '선택한 파일 없음';
    $('large-note').hidden = true;
    if (!candidate) { reading = false; status('HWPX 파일을 선택해 주세요.'); return; }
    try {
      if (!/\.hwpx$/i.test(candidate.name)) throw new Error('확장자가 .hwpx인 문서를 선택해 주세요.');
      if (candidate.size > 64 * 1024 * 1024) throw new Error('파일 크기가 64MiB 한도를 넘었습니다. 문서를 나누거나 CLI를 사용해 주세요.');
      const header = new Uint8Array(await candidate.slice(0, 4).arrayBuffer());
      if (ownGeneration !== generation) return;
      if (header.length !== 4 || header[0] !== 0x50 || header[1] !== 0x4b || header[2] !== 3 || header[3] !== 4) {
        throw new Error('ZIP 기반 HWPX 파일이 아닙니다. 한글에서 HWPX 형식으로 다시 저장해 주세요.');
      }
      file = candidate;
      $('large-note').hidden = false;
      status('파일이 준비되었습니다. 설정을 확인한 뒤 변환하세요.');
    } catch (error) {
      if (ownGeneration === generation) status(error.message, true);
    } finally {
      if (ownGeneration === generation) { reading = false; setBusy(false); }
    }
  }
  const updateGate = () => {
    if (!result) return;
    const allowed = result.metadata.diagnostics.length === 0 || $('ack').checked;
    $('publish-controls').disabled = !allowed;
    $('publish-gate').textContent = allowed ? '결과를 저장한 뒤 게시할 사이트에서 확인하세요.' : '경고를 원본과 대조하고 왼쪽의 확인 항목을 체크하면 게시 준비를 사용할 수 있습니다.';
    step(allowed ? 'publish' : 'preview');
  };
  const escaped = value => value.replace(/[&<>"']/g, char => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[char]));
  const height = () => Math.ceil(result.options.paged ? result.metadata.summary.paged_height_px : result.metadata.summary.document_height_px);
  const embedCodes = () => {
    if (!result) return;
    const summary = result.metadata.summary;
    const containerStyle = 'position:relative; isolation:isolate; min-height:' + height() + 'px;';
    $('div-code').value = '<div class="hwpx-viewer" style="' + containerStyle + '">\n  <!-- 내려받은 HTML 전체를 <html> 태그까지 그대로 이곳에 포함 -->\n</div>';
    const value = $('published-url').value.trim();
    $('copy-iframe').disabled = true;
    $('iframe-code').value = '';
    $('url-error').textContent = '';
    if (!value) return;
    try {
      const url = new URL(value);
      if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error('HTTP 또는 HTTPS 주소를 입력해 주세요.');
      const href = escaped(url.href), title = escaped(summary.title);
      $('iframe-code').value = '<a href="' + href + '" target="_blank" rel="noopener noreferrer">' + title + ' 원문 HTML 열기</a>\n<iframe src="' + href + '" title="' + title + '" style="width:100%; height:' + height() + 'px; border:0;" loading="lazy"></iframe>';
      $('copy-iframe').disabled = false;
    } catch (error) { $('url-error').textContent = '올바른 공개 HTML 주소를 입력해 주세요. 사용자명·암호가 포함된 주소는 사용할 수 없습니다.'; }
  };
  function diagnostics(metadata, paged) {
    $('warning-card').hidden = false;
    $('warning-count').textContent = '(' + metadata.diagnostics.length + ')';
    $('warning-empty').hidden = metadata.diagnostics.length !== 0;
    $('ack-label').hidden = metadata.status !== 'converted' || metadata.diagnostics.length === 0;
    const list = document.createDocumentFragment();
    for (const warning of metadata.diagnostics) {
      const item = document.createElement('li');
      const row = document.createElement('div'); row.className = 'warning-line';
      const tag = document.createElement('span'); tag.className = 'warning-tag'; tag.textContent = '경고'; row.append(tag);
      const location = document.createElement(warning.page && paged && previewUrl ? 'button' : 'span');
      location.textContent = warning.page ? warning.page + '쪽' : '위치 미상';
      if (location.tagName === 'BUTTON') {
        location.type = 'button'; location.setAttribute('aria-label', warning.page + '쪽 미리보기로 이동');
        location.addEventListener('click', () => { $('preview').src = previewUrl + '#page-' + warning.page; $('preview').focus(); });
      }
      row.append(location); item.append(row);
      for (const text of [warning.message, warning.action]) { const p = document.createElement('p'); p.textContent = text; item.append(p); }
      const detail = document.createElement('details'), label = document.createElement('summary'), source = document.createElement('p');
      label.textContent = '원문 사유와 위치 키'; source.textContent = warning.detail + (warning.key ? '\n' + warning.key : '');
      source.className = 'error-detail'; detail.append(label, source); item.append(detail); list.append(item);
    }
    $('warnings').replaceChildren(list);
  }
  function showResult(metadata, html, name, options) {
    if (metadata.status !== 'converted') {
      diagnostics(metadata, options.paged);
      status(metadata.error.message, true, metadata.error.detail);
      step('preview');
      return;
    }
    const filename = name.replace(/\.hwpx$/i, '') + '.html';
    const blob = new Blob([html], {type: 'text/html;charset=utf-8'});
    previewUrl = URL.createObjectURL(blob);
    result = {metadata, blob, filename, options};
    $('preview').src = previewUrl + (options.paged ? '#page-1' : '');
    $('preview').hidden = false;
    $('placeholder').hidden = true;
    $('result-name').textContent = filename;
    const summary = metadata.summary;
    const metrics = [['쪽 수', summary.page_count], ['그림 자원', summary.image_count], ['누락·단순화', summary.skipped_parts], ['HTML 크기', (blob.size / 1024 / 1024).toFixed(2) + ' MiB']];
    for (const [label, value] of metrics) {
      const group = document.createElement('div'), dt = document.createElement('dt'), dd = document.createElement('dd');
      dt.textContent = label; dd.textContent = value; group.append(dt, dd); $('metrics').append(group);
    }
    $('metrics').hidden = false;
    diagnostics(metadata, options.paged);
    $('publish-card').hidden = false;
    embedCodes(); updateGate();
    status('변환을 마쳤습니다. 미리보기' + (metadata.diagnostics.length ? '와 경고를 확인해 주세요.' : '를 원본과 대조한 뒤 결과를 저장하세요.'));
  }
  async function convert() {
    if (!file || busy || reading) return;
    const selected = file, ownGeneration = ++generation;
    const options = {paged: $('view').value === 'paged', infer: $('infer').checked, strict: $('strict').checked, readingView: $('reading-view').checked};
    clearResult(); setBusy(true);
    const started = performance.now();
    let stage = '파일을 읽고 있습니다';
    timer = setInterval(() => status(stage + ' · ' + Math.floor((performance.now() - started) / 1000) + '초'), 1000);
    status(stage + '…');
    try {
      const input = await selected.arrayBuffer();
      if (ownGeneration !== generation) return;
      const encoded = $('wasm-data').content.textContent.trim();
      const wasm = Uint8Array.from(atob(encoded), char => char.charCodeAt(0)).buffer;
      workerUrl = URL.createObjectURL(new Blob([$('worker-source').textContent], {type: 'text/javascript'}));
      worker = new Worker(workerUrl);
      worker.onmessage = ({data}) => {
        if (ownGeneration !== generation) return;
        if (data.stage) { stage = data.stage === 'engine' ? '변환 엔진을 준비하고 있습니다' : '문서를 변환하고 있습니다'; return; }
        stopWorker();
        if (data.failure) status(data.failure, true, data.detail);
        else showResult(data.metadata, data.html, selected.name, options);
      };
      worker.onerror = event => { if (ownGeneration === generation) { stopWorker(); status('변환 작업을 시작하지 못했습니다. 브라우저의 로컬 파일 실행 정책을 확인해 주세요.', true, event.message); } };
      worker.onmessageerror = () => { if (ownGeneration === generation) { stopWorker(); status('변환 결과를 받지 못했습니다. 다시 변환해 주세요.', true); } };
      const flags = (options.paged ? 1 : 0) | (options.infer ? 2 : 0) | (options.strict ? 4 : 0) | (options.readingView ? 8 : 0);
      worker.postMessage({input, wasm, name: selected.name, flags}, [input, wasm]);
    } catch (error) { if (ownGeneration === generation) { stopWorker(); status('변환 준비 중 오류가 발생했습니다. 파일을 다시 선택해 주세요.', true, String(error)); } }
  }
  async function copy(value, source) {
    if (!result || $('publish-controls').disabled) return;
    try {
      if (navigator.clipboard && window.isSecureContext) await navigator.clipboard.writeText(value);
      else throw new Error('clipboard unavailable');
      status('클립보드에 복사했습니다.');
    } catch (error) {
      const area = source || $('copy-fallback');
      if (!source) { area.value = value; $('manual-copy').hidden = false; $('manual-copy').open = true; }
      area.focus(); area.select();
      let copied = false;
      try { copied = document.execCommand('copy'); } catch (ignored) { /* Select for manual copying. */ }
      status(copied ? '클립보드에 복사했습니다.' : '자동 복사를 사용할 수 없습니다. 선택한 내용을 Ctrl+C로 복사하세요.');
    }
  }
  $('file').addEventListener('change', () => choose($('file').files[0]));
  $('drop').addEventListener('dragover', event => { event.preventDefault(); $('drop').classList.add('drag'); });
  $('drop').addEventListener('dragleave', () => $('drop').classList.remove('drag'));
  $('drop').addEventListener('drop', event => {
    event.preventDefault(); $('drop').classList.remove('drag');
    if (event.dataTransfer.files.length !== 1) { status('HWPX 파일 하나를 끌어 놓아 주세요.', true); return; }
    $('file').value = ''; choose(event.dataTransfer.files[0]);
  });
  // Dropping outside the target must not navigate away from the local tool.
  window.addEventListener('dragover', event => event.preventDefault());
  window.addEventListener('drop', event => event.preventDefault());
  $('settings').addEventListener('change', () => { clearResult(); status('설정이 바뀌었습니다. 같은 파일을 다시 변환하세요.'); });
  $('convert').addEventListener('click', convert);
  $('cancel').addEventListener('click', () => { ++generation; stopWorker(); clearResult(); status('변환을 중지했습니다. 다시 변환할 수 있습니다.'); });
  $('ack').addEventListener('change', updateGate);
  $('published-url').addEventListener('input', embedCodes);
  $('copy-iframe').addEventListener('click', () => copy($('iframe-code').value, $('iframe-code')));
  $('copy-div').addEventListener('click', () => { embedCodes(); copy($('div-code').value, $('div-code')); });
  $('copy-info').addEventListener('click', () => {
    if (!result) return;
    const info = {tool: 'hwpx2html', version: result.metadata.summary.version, source: file.name, input_sha256: result.metadata.summary.input_sha256,
      options: {resource_mode: 'embedded', page_navigation: result.options.paged, infer_structure: result.options.infer, strict: result.options.strict, adjust_letter_spacing: true, reading_view: result.options.readingView},
      warning_count: result.metadata.diagnostics.length, unsupported_objects: result.metadata.summary.unsupported_objects, skipped_parts: result.metadata.summary.skipped_parts};
    copy(JSON.stringify(info, null, 2));
  });
  $('download').addEventListener('click', () => {
    if (!result || $('publish-controls').disabled) return;
    const link = document.createElement('a'); link.href = previewUrl; link.download = result.filename;
    document.body.append(link); link.click(); link.remove();
    status('HTML 저장을 시작했습니다.');
  });
  window.addEventListener('pagehide', () => { stopWorker(); if (previewUrl) URL.revokeObjectURL(previewUrl); });
})();
