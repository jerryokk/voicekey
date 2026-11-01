// 模拟浏览器的信令握手：连 WS → 收 hello → 发一个最小 offer → 等 answer。
// 只验证本机 RTC 侧能否产出带 ICE 候选的 answer；不需要真的有人说话。
// 用法：node probe.mjs <ws 地址，含 ?t=token>
const url = process.argv[2];
if (!url) {
  console.error('用法：node probe.mjs ws://127.0.0.1:47800/ws?t=xxxx');
  process.exit(2);
}

// 音频 sendonly 的最小 offer。真正校验的是对端能否 set_remote_description 并回候选。
const offer = [
  'v=0',
  'o=- 0 0 IN IP4 127.0.0.1',
  's=-',
  't=0 0',
  'a=group:BUNDLE 0',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111',
  'c=IN IP4 0.0.0.0',
  'a=rtcp:9 IN IP4 0.0.0.0',
  'a=ice-ufrag:probe',
  'a=ice-pwd:probeprobeprobeprobeprobe',
  'a=ice-options:trickle',
  'a=fingerprint:sha-256 8F:1B:2C:3D:4E:5F:60:71:82:93:A4:B5:C6:D7:E8:F9:0A:1B:2C:3D:4E:5F:60:71:82:93:A4:B5:C6:D7:E8',
  'a=setup:actpass',
  'a=mid:0',
  'a=sendonly',
  'a=rtcp-mux',
  'a=rtpmap:111 opus/48000/2',
].join('\r\n') + '\r\n';

const ws = new WebSocket(url);
const timer = setTimeout(() => {
  console.log('超时：30 秒内没等到 answer');
  process.exit(1);
}, 30000);

ws.addEventListener('open', () => {
  console.log('WS 已连接');
});

ws.addEventListener('message', (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === 'hello') {
    console.log('hello.ice =', JSON.stringify(msg.ice));
    console.log('发送 offer…');
    ws.send(JSON.stringify({ op: 'offer', sdp: offer }));
  } else if (msg.type === 'answer') {
    const lines = msg.sdp.split(/\r?\n/);
    const candidates = lines.filter((l) => l.startsWith('a=candidate:'));
    console.log('收到 answer，' + lines.length + ' 行，候选 ' + candidates.length + ' 条');
    for (const c of candidates) console.log('  ' + c);
    console.log(candidates.length > 0 ? 'PASS' : 'FAIL：answer 里没有候选');
    clearTimeout(timer);
    ws.close();
    process.exit(candidates.length > 0 ? 0 : 1);
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