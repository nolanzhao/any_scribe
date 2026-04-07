const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open, save } = window.__TAURI__.dialog;

// ============================================
// State
// ============================================
let currentSource = null; // 'local' | 'douyin' | 'xhs' | 'xiaoyuzhou'
let selectedFilePath = null;
let result = null; // TranscribeResult
let isProcessing = false;

let needFfmpeg = false;
let needModel = false;
let downloadingWhat = null;

const SOURCE_META = {
  local:      { label: '本地文件', hint: '支持 MP4/MKV/AVI/MOV 等', placeholder: '' },
  douyin:     { label: '抖音',     hint: '粘贴分享链接或短链接', placeholder: '粘贴抖音分享内容...' },
  xhs:        { label: '小红书',   hint: '粘贴分享链接或短链接', placeholder: '粘贴小红书分享内容...' },
  xiaoyuzhou: { label: '小宇宙',   hint: '粘贴播客节目页面链接', placeholder: 'https://www.xiaoyuzhoufm.com/episode/...' },
};

// ============================================
// DOM
// ============================================
const $ = (id) => document.getElementById(id);

const $modelOverlay = $('model-overlay');
const $overlayTitle = $('overlay-title');
const $overlayDesc = $('overlay-desc');
const $overlayInfo = $('overlay-info');
const $btnDownload = $('btn-download');
const $downloadProgressArea = $('download-progress-area');
const $downloadProgressBar = $('download-progress-bar');
const $downloadStatus = $('download-status');

const $sourceGrid = $('source-grid');
const $dropZone = $('drop-zone');
const $urlZone = $('url-zone');
const $urlInput = $('url-input');
const $urlZoneTitle = $('url-zone-title');
const $urlZoneHint = $('url-zone-hint');

const $btnStartUrl = $('btn-start-url');

const $fileInfo = $('file-info');
const $fileName = $('file-name');
const $filePath = $('file-path');
const $btnChangeFile = $('btn-change-file');
const $btnStartLocal = $('btn-start-local');

const $taskInfo = $('task-info');
const $taskTitle = $('task-title');
const $taskUrl = $('task-url');
const $btnNewTask = $('btn-new-task');

const $languageSelect = $('language-select');
const $progressSection = $('progress-section');
const $progressStage = $('progress-stage');
const $progressPercent = $('progress-percent');
const $transcribeProgressBar = $('transcribe-progress-bar');
const $progressMessage = $('progress-message');
const $btnCancel = $('btn-cancel');

const $resultsSection = $('results-section');
const $segmentCount = $('segment-count');
const $totalDuration = $('total-duration');
const $timeline = $('timeline');
const $btnOpenMedia = $('btn-open-media');
const $btnOpenFolder = $('btn-open-folder');
const $btnExportSrt = $('btn-export-srt');
const $btnExportTxt = $('btn-export-txt');
const $btnCopyAll = $('btn-copy-all');
const $completionBanner = $('completion-banner');
const $completionMessage = $('completion-message');
const $emptyState = $('empty-state');
const $btnSelectFile = $('btn-select-file');


const ICON = {
  play: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="5 3 19 12 5 21 5 3"/></svg>',
  loading: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="loading-spin"><path d="M12 2v4m0 12v4m-7.07-14.93l2.83 2.83m8.48 8.48l2.83 2.83M2 12h4m12 0h4M4.93 19.07l2.83-2.83m8.48-8.48l2.83-2.83"/></svg>',
  check: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>',
  copy: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>',
};

// ============================================
// Init
// ============================================
async function init() {
  initTheme();
  setupEventListeners();
  setupDragDrop();
  await setupTauriListeners();
  await checkDependencies();
}

async function checkDependencies() {
  const [ffmpegOk, modelInfo] = await Promise.all([
    invoke('check_ffmpeg'),
    invoke('check_model_status'),
  ]);
  needFfmpeg = !ffmpegOk;
  needModel = !modelInfo.exists;
  showNextDependencyOverlay();
}

function showNextDependencyOverlay() {
  $downloadProgressArea.classList.add('hidden');
  $downloadProgressBar.style.width = '0%';
  $btnDownload.disabled = false;

  if (needFfmpeg) {
    downloadingWhat = 'ffmpeg';
    $overlayTitle.textContent = '需要下载环境依赖';
    $overlayDesc.textContent = '首次使用需下载音视频处理组件 (FFmpeg)';
    $overlayInfo.innerHTML = '<div class="model-name">FFmpeg (macOS aarch64)</div><div class="model-size">~30 MB · 用于处理音视频</div>';
    $btnDownload.innerHTML = `${ICON.play} <span id="btn-download-text">下载依赖</span>`;
    $modelOverlay.classList.remove('hidden');
  } else if (needModel) {
    downloadingWhat = 'model';
    $overlayTitle.textContent = '需要下载语音模型';
    $overlayDesc.textContent = '首次使用需要下载 Whisper 语音识别模型';
    $overlayInfo.innerHTML = '<div class="model-name">ggml-large-v3-turbo</div><div class="model-size">~1.5 GB · 支持中英文 · 高精度</div>';
    $btnDownload.innerHTML = `${ICON.play} <span id="btn-download-text">下载模型</span>`;
    $modelOverlay.classList.remove('hidden');
  } else {
    $modelOverlay.classList.add('hidden');
  }
}

// ============================================
// Source Switching
// ============================================
function selectSource(source) {
  if (isProcessing) return;
  currentSource = source;

  // Highlight selected card
  document.querySelectorAll('.source-card').forEach(card => {
    card.classList.toggle('active', card.dataset.source === source);
  });

  // Hide all input zones
  $dropZone.classList.add('hidden');
  $urlZone.classList.add('hidden');
  $fileInfo.classList.add('hidden');
  $taskInfo.classList.add('hidden');
  $resultsSection.classList.add('hidden');
  $progressSection.classList.add('hidden');
  $completionBanner.classList.add('hidden');
  $btnOpenFolder.classList.add('hidden');
  $btnOpenMedia.classList.add('hidden');
  $emptyState.classList.add('hidden');
  result = null;

  if (source === 'local') {
    if (selectedFilePath) {
      $fileInfo.classList.remove('hidden');
    } else {
      $dropZone.classList.remove('hidden');
    }
  } else {
    const meta = SOURCE_META[source];
    $urlZoneTitle.textContent = `粘贴${meta.label}链接`;
    $urlZoneHint.textContent = meta.hint;
    $urlInput.placeholder = meta.placeholder;
    $urlInput.value = '';
    $urlZone.classList.remove('hidden');
  }
}

// ============================================
// Theme
// ============================================
function initTheme() {
  const saved = localStorage.getItem('anyscribe-theme');
  if (saved) { setTheme(saved); }
  else {
    setTheme(window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  }
}
function toggleTheme() {
  const cur = document.documentElement.getAttribute('data-theme') || 'light';
  const next = cur === 'dark' ? 'light' : 'dark';
  setTheme(next);
  localStorage.setItem('anyscribe-theme', next);
}
function setTheme(theme) {
  document.documentElement.setAttribute('data-theme', theme);
  $('icon-sun').classList.toggle('hidden', theme === 'dark');
  $('icon-moon').classList.toggle('hidden', theme !== 'dark');
}

// ============================================
// Downloads
// ============================================
async function handleDownload() {
  $btnDownload.disabled = true;
  $btnDownload.innerHTML = `${ICON.loading} <span>下载中...</span>`;
  $downloadProgressArea.classList.remove('hidden');
  try {
    if (downloadingWhat === 'ffmpeg') { await invoke('download_ffmpeg'); needFfmpeg = false; }
    else if (downloadingWhat === 'model') { await invoke('download_model'); needModel = false; }
    $downloadStatus.textContent = '下载完成';
    $downloadProgressBar.style.width = '100%';
    setTimeout(() => showNextDependencyOverlay(), 800);
  } catch (err) {
    $downloadStatus.textContent = `下载失败: ${err}`;
    $btnDownload.disabled = false;
    $btnDownload.innerHTML = `${ICON.play} <span>重新下载</span>`;
  }
}

// ============================================
// Local File
// ============================================
async function selectFile() {
  const selected = await open({
    multiple: false,
    filters: [{ name: 'Video', extensions: ['mp4','mkv','avi','mov','wmv','flv','webm','m4v','ts','mts'] }],
  });
  if (selected) setSelectedFile(selected);
}

function setSelectedFile(path) {
  selectedFilePath = path;
  $fileName.textContent = path.split('/').pop();
  $filePath.textContent = path;
  $dropZone.classList.add('hidden');
  $fileInfo.classList.remove('hidden');
  $resultsSection.classList.add('hidden');
  $progressSection.classList.add('hidden');
  result = null;
}

async function startLocalTranscribe() {
  if (!selectedFilePath || isProcessing) return;
  beginProcessing();
  $btnStartLocal.disabled = true;
  $btnStartLocal.innerHTML = `${ICON.loading} 转录中...`;

  try {
    const r = await invoke('transcribe_video', {
      videoPath: selectedFilePath,
      language: $languageSelect.value || null,
    });
    result = r;
    showResults(r);
  } catch (err) {
    showError(err);
  } finally {
    isProcessing = false;
    $btnCancel.classList.add('hidden');
    $btnStartLocal.disabled = false;
    $btnStartLocal.innerHTML = `${ICON.play} 开始转录`;
  }
}

// ============================================
// URL Processing
// ============================================
async function startUrlProcess() {
  const url = $urlInput.value.trim();
  if (!url || isProcessing) {
    $urlInput.classList.add('invalid');
    setTimeout(() => $urlInput.classList.remove('invalid'), 1500);
    return;
  }

  beginProcessing();
  $btnStartUrl.disabled = true;
  $btnStartUrl.innerHTML = `${ICON.loading} 处理中...`;

  // Show task info bar
  $urlZone.classList.add('hidden');
  $taskInfo.classList.remove('hidden');
  $taskTitle.textContent = '正在获取信息...';
  $taskUrl.textContent = url;

  try {
    const r = await invoke('process_url', {
      url,
      language: $languageSelect.value || null,
    });
    result = r;
    if (r.title) $taskTitle.textContent = r.title;
    showResults(r);
  } catch (err) {
    showError(err);
  } finally {
    isProcessing = false;
    $btnCancel.classList.add('hidden');
    $btnStartUrl.disabled = false;
    $btnStartUrl.innerHTML = `${ICON.play} 开始`;
  }
}

// ============================================
// Shared UI
// ============================================
function beginProcessing() {
  isProcessing = true;
  $progressSection.classList.remove('hidden');
  $emptyState.classList.add('hidden');
  $completionBanner.classList.add('hidden');
  $btnOpenFolder.classList.add('hidden');
  $btnOpenMedia.classList.add('hidden');

  $resultsSection.classList.remove('hidden');
  $timeline.innerHTML = '';
  $segmentCount.textContent = '0 段';
  $totalDuration.textContent = '00:00';

  $btnCancel.classList.remove('hidden');
  $btnCancel.disabled = false;
  $btnCancel.innerHTML = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg> 取消`;

  // Collapse source grid during processing
  $sourceGrid.classList.add('collapsed');
}

function showResults(r) {
  $progressSection.classList.add('hidden');
  $segmentCount.textContent = `${r.segments.length} 段`;
  $totalDuration.textContent = formatDuration(r.duration);

  $timeline.innerHTML = '';
  r.segments.forEach(seg => $timeline.appendChild(createSegmentEl(seg)));

  $completionBanner.classList.remove('hidden');
  const txtName = r.txt_path.split('/').pop();
  $completionMessage.textContent = `已自动保存: ${txtName}`;

  $btnOpenFolder.classList.remove('hidden');
  if (r.media_path) $btnOpenMedia.classList.remove('hidden');

  $sourceGrid.classList.remove('collapsed');
}

function showError(err) {
  $progressStage.innerHTML = `${ICON.check} 处理失败`;
  $progressMessage.textContent = typeof err === 'string' ? err : JSON.stringify(err);
  $progressPercent.textContent = '';
  $transcribeProgressBar.style.width = '0%';
  $sourceGrid.classList.remove('collapsed');
}

function resetView() {
  $taskInfo.classList.add('hidden');
  $resultsSection.classList.add('hidden');
  $progressSection.classList.add('hidden');
  $completionBanner.classList.add('hidden');
  $emptyState.classList.add('hidden');
  result = null;
  if (currentSource) selectSource(currentSource);
}

function createSegmentEl(segment) {
  const div = document.createElement('div');
  div.className = 'segment';
  div.innerHTML = `<span class="segment-time">${formatTime(segment.start)} → ${formatTime(segment.end)}</span><span class="segment-text">${escapeHtml(segment.text)}</span>`;
  return div;
}

// ============================================
// File Operations
// ============================================
async function openFolder() {
  if (!result) return;
  try { await invoke('open_containing_folder', { path: result.srt_path }); } catch(e) { console.error(e); }
}
async function openMedia() {
  if (!result?.media_path) return;
  try { await invoke('open_file', { path: result.media_path }); } catch(e) { console.error(e); }
}
async function exportSrt() {
  if (!result) return;
  const srt = result.segments.map((s,i) => `${i+1}\n${formatSrtTs(s.start)} --> ${formatSrtTs(s.end)}\n${s.text}\n`).join('\n');
  const path = await save({ filters: [{ name: 'SRT', extensions: ['srt'] }], defaultPath: getExportName('srt') });
  if (path) await invoke('save_file', { path, content: srt });
}
async function exportTxt() {
  if (!result) return;
  const txt = result.segments.map(s => `[${formatTime(s.start)} --> ${formatTime(s.end)}] ${s.text}`).join('\n');
  const path = await save({ filters: [{ name: 'TXT', extensions: ['txt'] }], defaultPath: getExportName('txt') });
  if (path) await invoke('save_file', { path, content: txt });
}
async function copyAll() {
  if (!result) return;
  const text = result.segments.map(s => s.text).join('\n');
  try {
    await navigator.clipboard.writeText(text);
    $btnCopyAll.innerHTML = `${ICON.check} 已复制`;
    setTimeout(() => { $btnCopyAll.innerHTML = `${ICON.copy} 复制`; }, 1500);
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text; document.body.appendChild(ta); ta.select(); document.execCommand('copy'); document.body.removeChild(ta);
  }
}

// ============================================
// Events
// ============================================
function setupEventListeners() {
  $btnDownload.addEventListener('click', handleDownload);

  // Source cards
  document.querySelectorAll('.source-card').forEach(card => {
    card.addEventListener('click', () => selectSource(card.dataset.source));
  });

  // Local
  $btnSelectFile.addEventListener('click', selectFile);
  $dropZone.addEventListener('click', (e) => { if (e.target === $dropZone || e.target.tagName === 'H3' || e.target.tagName === 'P' || e.target.tagName === 'svg' || e.target.tagName === 'path' || e.target.tagName === 'rect') selectFile(); });
  $btnChangeFile.addEventListener('click', () => { selectedFilePath = null; selectSource('local'); });
  $btnStartLocal.addEventListener('click', startLocalTranscribe);

  // URL

  $btnStartUrl.addEventListener('click', startUrlProcess);
  $urlInput.addEventListener('keydown', e => { if (e.key === 'Enter') startUrlProcess(); $urlInput.classList.remove('invalid'); });
  $btnNewTask.addEventListener('click', resetView);

  // Shared
  $btnOpenFolder.addEventListener('click', openFolder);
  $btnOpenMedia.addEventListener('click', openMedia);
  $btnExportSrt.addEventListener('click', exportSrt);
  $btnExportTxt.addEventListener('click', exportTxt);
  $btnCopyAll.addEventListener('click', copyAll);
  $('btn-theme').addEventListener('click', toggleTheme);


  $btnCancel.addEventListener('click', async () => {
    $btnCancel.disabled = true;
    $btnCancel.innerHTML = `${ICON.loading} 中断中...`;
    try { await invoke('cancel_transcription'); } catch(e) { console.error(e); }
  });
}

function setupDragDrop() {
  ['dragenter','dragover','dragleave','drop'].forEach(ev => {
    document.body.addEventListener(ev, e => { e.preventDefault(); e.stopPropagation(); });
  });
  $dropZone.addEventListener('dragenter', () => $dropZone.classList.add('drag-over'));
  $dropZone.addEventListener('dragleave', () => $dropZone.classList.remove('drag-over'));
  $dropZone.addEventListener('dragover', e => { e.preventDefault(); $dropZone.classList.add('drag-over'); });
  $dropZone.addEventListener('drop', e => {
    e.preventDefault(); $dropZone.classList.remove('drag-over');
    const files = e.dataTransfer?.files;
    if (files?.length > 0 && files[0].path) setSelectedFile(files[0].path);
  });
}

async function setupTauriListeners() {
  await listen('transcribe-progress', event => {
    const d = event.payload;
    const pct = Math.round(d.progress * 100);
    $progressStage.innerHTML = `${getStageIcon(d.stage)} ${getStageLabel(d.stage)}`;
    $progressPercent.textContent = `${pct}%`;
    $transcribeProgressBar.style.width = `${pct}%`;
    $progressMessage.textContent = d.message;

    // Update task title from progress
    if (d.stage === 'downloading' && d.message) {
      const m = d.message.match(/^正在下载: (.+)\.\.\./);
      if (m) $taskTitle.textContent = m[1];
    }

    // Real-time segments
    if (d.segment && d.stage === 'transcribing') {
      const idx = $timeline.children.length;
      $timeline.appendChild(createSegmentEl(d.segment));
      $segmentCount.textContent = `${idx + 1} 段`;
      $timeline.scrollTop = $timeline.scrollHeight;
    }
  });

  await listen('download-progress', event => {
    const d = event.payload;
    if (d.total > 0) $downloadProgressBar.style.width = `${Math.round((d.downloaded / d.total) * 100)}%`;
    $downloadStatus.textContent = d.message;
  });
}

// ============================================
// Helpers
// ============================================
function formatTime(s) {
  const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), sec=Math.floor(s%60), ms=Math.round((s%1)*1000);
  return h>0 ? `${p(h)}:${p(m)}:${p(sec)}.${p3(ms)}` : `${p(m)}:${p(sec)}.${p3(ms)}`;
}
function formatSrtTs(s) {
  const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), sec=Math.floor(s%60), ms=Math.round((s%1)*1000);
  return `${p(h)}:${p(m)}:${p(sec)},${p3(ms)}`;
}
function formatDuration(s) {
  if (!s) return '00:00';
  const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), sec=Math.floor(s%60);
  return h>0 ? `${p(h)}:${p(m)}:${p(sec)}` : `${p(m)}:${p(sec)}`;
}
function p(n) { return n.toString().padStart(2,'0'); }
function p3(n) { return n.toString().padStart(3,'0'); }

function getStageIcon(stage) {
  const spinner = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="loading-spin"><path d="M12 2v4m0 12v4m-7.07-14.93l2.83 2.83m8.48 8.48l2.83 2.83M2 12h4m12 0h4M4.93 19.07l2.83-2.83m8.48-8.48l2.83-2.83"/></svg>';
  const check = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>';
  return stage === 'done' ? check : spinner;
}

function getStageLabel(stage) {
  return { extracting:'提取音频', fetching:'解析链接', downloading:'下载中', converting:'转换格式', loading:'加载模型', loading_audio:'加载音频', transcribing:'正在转录', done:'转录完成' }[stage] || stage;
}

function getExportName(ext) {
  if (currentSource === 'local' && selectedFilePath) return selectedFilePath.split('/').pop().replace(/\.[^/.]+$/, '') + '.' + ext;
  if (result?.title) return result.title + '.' + ext;
  return `transcription.${ext}`;
}

function escapeHtml(str) { const d = document.createElement('div'); d.textContent = str; return d.innerHTML; }


// ============================================
// Start
// ============================================
init();
