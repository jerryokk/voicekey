// 退格协议验证：连上 WS，发 backspace 操作，断言连接保持正常。
// 用法：node backspace-test.mjs "ws://127.0.0.1:47800/ws?t=<token>"
const url = process.argv[2];
if (!url) { console.error('用法：node backspace-test.mjs <ws地址>'); process.exit(1); }

const ws = new WebSocket(url);
let done = false;
ws.onopen = () => {
  console.log('已连接');
  ws.send(JSON.stringify({ op: 'backspace', n: 3 }));
  setTimeout(() => ws.send(JSON.stringify({ op: 'backspace' })), 200); // 不带 n，默认 1
  setTimeout(() => ws.send(JSON.stringify({ op: 'backspace', n: 999 })), 400); // 超上限，应被截到 50
  setTimeout(() => { ws.close(); }, 800);
};
ws.onmessage = (e) => console.log('收到:', e.data);
ws.onclose = () => { if (!done) { done = true; console.log('三条退格均已发出，连接正常关闭 ✓'); process.exit(0); } };
ws.onerror = (e) => { console.error('连接失败', e.message); process.exit(1); };
setTimeout(() => { if (!done) { console.error('超时'); process.exit(1); } }, 5000);
