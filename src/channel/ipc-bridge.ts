import { EventEmitter } from "node:events";
import {
  createServer,
  createConnection,
  Server,
  Socket,
} from "node:net";
import { unlinkSync, existsSync, chmodSync } from "node:fs";

function encode(msg: unknown): string {
  return JSON.stringify(msg) + "\n";
}

const MAX_LINE_BUFFER = 10 * 1024 * 1024; // 10 MB

function makeLineParser(onMessage: (msg: unknown) => void, onOverflow?: () => void) {
  let buf = "";
  return (data: Buffer | string) => {
    buf += data.toString();
    if (buf.length > MAX_LINE_BUFFER) {
      buf = "";
      onOverflow?.();
      return;
    }
    const lines = buf.split("\n");
    // Last element is either empty string or an incomplete line
    buf = lines.pop() ?? "";
    for (const line of lines) {
      if (line.trim() === "") continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue; // truly malformed JSON, skip this line
      }
      onMessage(msg);
    }
  };
}

export class IpcServer extends EventEmitter {
  private sockPath: string;
  private server: Server | null = null;
  private clients: Set<Socket> = new Set();
  private secret: string | undefined;
  private logger?: { warn(obj: unknown, msg?: string): void; debug(obj: unknown, msg?: string): void };

  constructor(sockPath: string, logger?: { warn(obj: unknown, msg?: string): void; debug(obj: unknown, msg?: string): void }, secret?: string) {
    super();
    this.sockPath = sockPath;
    this.logger = logger;
    this.secret = secret;
  }

  async listen(): Promise<void> {
    // Clean up stale socket file if it exists
    if (existsSync(this.sockPath)) {
      try {
        unlinkSync(this.sockPath);
      } catch {
        // Ignore if already gone — race with another process
      }
    }

    return new Promise((resolve, reject) => {
      this.server = createServer((socket) => {
        if (!this.secret) {
          // No secret configured — accept immediately
          this.acceptClient(socket);
          return;
        }
        // Require auth handshake as first message
        const authTimeout = setTimeout(() => {
          this.logger?.warn("IPC auth timeout, dropping client");
          socket.destroy();
        }, 5000);
        const authParse = makeLineParser((msg) => {
          clearTimeout(authTimeout);
          const m = msg as Record<string, unknown>;
          if (m.type === "auth" && m.secret === this.secret) {
            socket.removeAllListeners("data");
            this.acceptClient(socket);
          } else {
            this.logger?.warn("IPC auth failed, dropping client");
            socket.destroy();
          }
        });
        socket.on("data", authParse);
        socket.on("error", () => {
          clearTimeout(authTimeout);
          socket.destroy();
        });
      });

      this.server.on("error", reject);
      this.server.listen(this.sockPath, () => {
        try { chmodSync(this.sockPath, 0o600); } catch { /* best-effort */ }
        resolve();
      });
    });
  }

  private acceptClient(socket: Socket): void {
    this.clients.add(socket);
    const parse = makeLineParser((msg) => {
      this.emit("message", msg, socket);
    }, () => {
      this.logger?.warn("IPC buffer overflow, dropping client");
      socket.destroy();
      this.clients.delete(socket);
    });
    socket.on("data", parse);
    socket.on("close", () => {
      this.clients.delete(socket);
    });
    socket.on("error", (err) => {
      this.logger?.warn({ err }, "IPC client socket error, removing client");
      this.clients.delete(socket);
    });
  }

  broadcast(msg: unknown): void {
    const data = encode(msg);
    for (const client of this.clients) {
      if (!client.destroyed) {
        client.write(data);
      }
    }
  }

  send(socket: Socket, msg: unknown): void {
    if (!socket.destroyed) {
      socket.write(encode(msg));
    }
  }

  async close(): Promise<void> {
    for (const client of this.clients) {
      client.destroy();
    }
    this.clients.clear();

    return new Promise((resolve) => {
      if (!this.server) {
        resolve();
        return;
      }
      this.server.close(() => {
        try {
          if (existsSync(this.sockPath)) {
            unlinkSync(this.sockPath);
          }
        } catch {
          // Best-effort socket file cleanup on shutdown
        }
        resolve();
      });
    });
  }
}

export class IpcClient extends EventEmitter {
  private sockPath: string;
  private secret: string | undefined;
  private socket: Socket | null = null;

  constructor(sockPath: string, secret?: string) {
    super();
    this.sockPath = sockPath;
    this.secret = secret;
  }

  async connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const socket = createConnection(this.sockPath);
      this.socket = socket;

      const parse = makeLineParser((msg) => {
        this.emit("message", msg);
      });

      socket.on("data", parse);
      socket.on("error", (err) => {
        this.emit("disconnect", err);
      });
      socket.on("close", () => {
        this.emit("disconnect", new Error("socket closed"));
      });
      socket.once("error", reject);
      socket.once("connect", () => {
        // Remove the one-shot error handler used for connection failure
        socket.removeListener("error", reject);
        // Send auth handshake if secret is configured
        if (this.secret) {
          socket.write(encode({ type: "auth", secret: this.secret }));
        }
        resolve();
      });
    });
  }

  get connected(): boolean {
    return this.socket != null && !this.socket.destroyed;
  }

  send(msg: unknown): void {
    if (this.socket && !this.socket.destroyed) {
      this.socket.write(encode(msg));
    }
  }

  async close(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.socket || this.socket.destroyed) {
        resolve();
        return;
      }
      this.socket.once("close", resolve);
      this.socket.destroy();
    });
  }
}
