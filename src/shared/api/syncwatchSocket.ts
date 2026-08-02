import {
  parseServerMessage,
  type ClientMessage,
  type ServerMessage,
} from "./protocol";

type MessageListener = (message: ServerMessage) => void;
type StatusListener = (status: "connected" | "disconnected") => void;

export class SyncWatchSocket {
  private socket: WebSocket | null = null;

  constructor(
    private readonly onMessage: MessageListener,
    private readonly onStatus: StatusListener,
  ) {}

  connect(url: string): Promise<void> {
    this.disconnect();

    return new Promise((resolve, reject) => {
      const socket = new WebSocket(url);
      this.socket = socket;

      socket.addEventListener("open", () => {
        this.onStatus("connected");
        resolve();
      });
      socket.addEventListener("message", (event) => {
        if (typeof event.data !== "string") return;
        const message = parseServerMessage(event.data);
        if (message) this.onMessage(message);
      });
      socket.addEventListener("close", () => this.onStatus("disconnected"));
      socket.addEventListener("error", () => {
        if (socket.readyState !== WebSocket.OPEN) {
          reject(new Error("Не удалось подключиться к комнате"));
        }
      });
    });
  }

  send(message: ClientMessage): void {
    if (this.socket?.readyState !== WebSocket.OPEN) {
      throw new Error("Связь с комнатой потеряна");
    }
    this.socket.send(JSON.stringify(message));
  }

  disconnect(): void {
    this.socket?.close();
    this.socket = null;
  }
}

export function normalizeWsUrl(address: string): string {
  let normalized = address.trim().replace(/\/$/, "");
  if (!/^wss?:\/\//i.test(normalized)) normalized = `ws://${normalized}`;

  const url = new URL(normalized);
  if (!url.port) {
    throw new Error("Укажите адрес комнаты полностью");
  }
  if (url.pathname === "/") url.pathname = "/ws";
  return url.toString();
}
