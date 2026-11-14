// WebSocket client over Bun's built-in WebSocket. Usage: bun ws-client.ts URL SECONDS < messages
// Every JSON line on stdin is sent as a message; end of input closes the socket. Every received
// message is printed as one JSON line, with frames reduced to {type, width, height, bytes,
// deviceWidth, deviceHeight} so a log stays small. Exits on close or after SECONDS.
const [url, secondsText] = process.argv.slice(2);
const socket = new WebSocket(url);
const out = (value: unknown) => console.log(JSON.stringify(value));

// Pixel size of a base64 PNG or JPEG.
function imageSize(base64: string): { width?: number; height?: number; bytes: number } {
  const bytes = Buffer.from(base64, "base64");
  if (bytes.readUInt32BE(0) === 0x89504e47) {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), bytes: bytes.length };
  }
  let at = 2;
  while (at + 9 < bytes.length) {
    if (bytes[at] !== 0xff) break;
    const marker = bytes[at + 1];
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      return { height: bytes.readUInt16BE(at + 5), width: bytes.readUInt16BE(at + 7), bytes: bytes.length };
    }
    at += 2 + bytes.readUInt16BE(at + 2);
  }
  return { bytes: bytes.length };
}

socket.onmessage = (event) => {
  const message = JSON.parse(String(event.data));
  if (message.type === "frame") {
    const { width, height, bytes } = imageSize(message.data);
    out({
      type: "frame",
      width,
      height,
      bytes,
      deviceWidth: message.metadata?.deviceWidth,
      deviceHeight: message.metadata?.deviceHeight,
    });
  } else {
    out(message);
  }
};
socket.onclose = () => {
  out({ type: "socket_closed" });
  process.exit(0);
};
socket.onerror = () => {
  out({ type: "socket_error" });
  process.exit(1);
};
setTimeout(() => process.exit(0), Number(secondsText) * 1000);

socket.onopen = async () => {
  let pending = "";
  for await (const chunk of Bun.stdin.stream()) {
    pending += Buffer.from(chunk).toString();
    const lines = pending.split("\n");
    pending = lines.pop() ?? "";
    for (const line of lines) if (line.trim()) socket.send(line);
  }
  socket.close();
};
