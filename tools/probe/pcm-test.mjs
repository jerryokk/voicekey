// 验证回落通道：把 WAV 拆成 16kHz Int16LE 的 20ms 帧，走 WS 二进制发进去。
// 这条路径不经过 WebRTC，正是打洞失败时要用的那条。
// 用法：node pcm-test.mjs <ws 地址含 token> <wav 路径>
const [, , url, wavPath] = process.argv;
if (!url || !wavPath) {
  console.error('用法：node pcm-test.mjs ws://127.0.0.1:47800/ws?t=xxxx ../ios/Resources/sample.wav');
  process.exit(2);
}

import fs from 'node:fs';

// 只处理 16 位单声道 WAV：和页面发出来的格式一致，不需要再重采样
function readPcm16(path) {
  const buf = fs.readFileSync(path);
  if (buf.toString('ascii', 0, 4) !== 'RIFF' || buf.toString('ascii', 8, 12) !== 'WAVE') {
    throw new Error('不是 RIFF/WAVE 文件');
  }
  // 不能假设 fmt 就在固定偏移：WAVE_FORMAT_EXTENSIBLE 会多出扩展字段
  let channels = 0, rate = 0, bits = 0, data = null;
  let offset = 12;
  while (offset + 8 <= buf.length) {
    const id = buf.toString('ascii', offset, offset + 4);
    const size = buf.readUInt32LE(offset + 4);
    const body = offset + 8;
    if (id === 'fmt ') {
      channels = buf.readUInt16LE(body + 2);
      rate = buf.readUInt32LE(body + 4);
      bits = buf.readUInt16LE(body + 14);
    } else if (id === 'data') {
      data = buf.subarray(body, body + size);
    }
    offset = body + size + (size % 2);
  }
  if (!data) throw new Error('WAV 里找不到 data 块');
  if (rate !== 16000 || channels !== 1 || bits !== 16) {
    throw new Error(`需要 16kHz/单声道/16bit，实际 ${rate}/${channels}/${bits}`);
  }
  return data;
}

const pcm = readPcm16(wavPath);
const frameBytes = 320 * 2;
const ws = new WebSocket(url);
const started = Date.now();
let sent = 0;

const timer = setTimeout(() => {
  console.log('超时：30 秒内没有定稿');
  process.exit(1);
}, 30000);

ws.addEventListener('message', async (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === 'hello') {
    console.log('开始走回落通道，' + (pcm.length / frameBytes).toFixed(0) + ' 帧');
    ws.send(JSON.stringify({ op: 'start' }));
    // 按实时节奏推流：引擎是流式的，灌太快会被服务端 VAD 当成异常
    for (let i = 0; i + frameBytes <= pcm.length; i += frameBytes) {
      if (ws.readyState !== 1) break;
      ws.send(pcm.subarray(i, i + frameBytes));
      sent += frameBytes;
      await new Promise((r) => setTimeout(r, 20));
    }
    ws.send(JSON.stringify({ op: 'stop' }));
  } else if (msg.type === 'partial') {
    console.log('… ' + msg.text);
  } else if (msg.type === 'stats') {
    console.log('统计：' + msg.text);
  } else if (msg.type === 'final') {
    const seconds = ((Date.now() - started) / 1000).toFixed(1);
    console.log('');
    console.log('FINAL: ' + msg.text);
    console.log(`发了 ${(sent / 1024).toFixed(0)} kB，用时 ${seconds} 秒`);
    clearTimeout(timer);
    ws.close();
    process.exit(msg.text && msg.text.length > 0 ? 0 : 1);
  } else if (msg.type === 'error') {
    console.log('ERROR ' + msg.stage + '：' + msg.message);
    clearTimeout(timer);
    process.exit(1);
  }
});

ws.addEventListener('error', (e) => {
  console.log('WS 出错：' + (e && e.message ? e.message : e));
  process.exit(1);
});