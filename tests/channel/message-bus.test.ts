import { describe, it, expect, vi, beforeEach } from "vitest";
import { EventEmitter } from "node:events";
import { MessageBus } from "../../src/channel/message-bus.js";
import type { ChannelAdapter, InboundMessage, ApprovalHandle, PermissionPrompt } from "../../src/channel/types.js";

function makeAdapter(id: string, type = "mock"): ChannelAdapter & EventEmitter {
  const emitter = new EventEmitter() as ChannelAdapter & EventEmitter;
  (emitter as any).id = id;
  (emitter as any).type = type;
  emitter.start = vi.fn(async () => {});
  emitter.stop = vi.fn(async () => {});
  emitter.sendText = vi.fn(async () => ({ messageId: "m1", chatId: "c1" }));
  emitter.sendFile = vi.fn(async () => ({ messageId: "m2", chatId: "c1" }));
  emitter.editMessage = vi.fn(async () => {});
  emitter.react = vi.fn(async () => {});
  emitter.downloadAttachment = vi.fn(async () => "/tmp/file");
  emitter.handlePairing = vi.fn(async () => "code");
  emitter.confirmPairing = vi.fn(async () => true);
  emitter.sendApproval = vi.fn(async (_prompt: PermissionPrompt, _cb: (d: "approve" | "deny") => void, _signal?: AbortSignal): Promise<ApprovalHandle> => {
    return { cancel: vi.fn() };
  });
  return emitter;
}

describe("MessageBus", () => {
  let bus: MessageBus;

  beforeEach(() => {
    bus = new MessageBus();
  });

  describe("adapter registry", () => {
    it("registers and retrieves adapters", () => {
      const adapter = makeAdapter("a1");
      bus.register(adapter);
      expect(bus.getAdapter("a1")).toBe(adapter);
    });

    it("unregisters adapters", () => {
      const adapter = makeAdapter("a1");
      bus.register(adapter);
      bus.unregister("a1");
      expect(bus.getAdapter("a1")).toBeUndefined();
    });

    it("getAllAdapters returns all registered adapters", () => {
      const a1 = makeAdapter("a1");
      const a2 = makeAdapter("a2");
      bus.register(a1);
      bus.register(a2);
      const all = bus.getAllAdapters();
      expect(all).toHaveLength(2);
      expect(all).toContain(a1);
      expect(all).toContain(a2);
    });
  });

  describe("inbound message merge", () => {
    it("re-emits messages from both adapters on the bus", async () => {
      const a1 = makeAdapter("a1");
      const a2 = makeAdapter("a2");
      bus.register(a1);
      bus.register(a2);

      const received: InboundMessage[] = [];
      bus.on("message", (msg: InboundMessage) => received.push(msg));

      const msg1: InboundMessage = {
        source: "mock", adapterId: "a1", chatId: "c1", messageId: "m1",
        userId: "u1", username: "user1", text: "hello from a1", timestamp: new Date(),
      };
      const msg2: InboundMessage = {
        source: "mock", adapterId: "a2", chatId: "c2", messageId: "m2",
        userId: "u2", username: "user2", text: "hello from a2", timestamp: new Date(),
      };

      a1.emit("message", msg1);
      a2.emit("message", msg2);

      expect(received).toHaveLength(2);
      expect(received[0]).toBe(msg1);
      expect(received[1]).toBe(msg2);
    });
  });

  describe("outbound routing", () => {
    it("routes to specific adapter when adapterId is set", async () => {
      const a1 = makeAdapter("a1");
      const a2 = makeAdapter("a2");
      bus.register(a1);
      bus.register(a2);

      await bus.send({ adapterId: "a1", chatId: "c1" }, { text: "hello" });

      expect(a1.sendText).toHaveBeenCalledWith("c1", "hello", expect.objectContaining({}));
      expect(a2.sendText).not.toHaveBeenCalled();
    });

    it("broadcasts to all adapters when no adapterId", async () => {
      const a1 = makeAdapter("a1");
      const a2 = makeAdapter("a2");
      bus.register(a1);
      bus.register(a2);

      await bus.send({ chatId: "c1" }, { text: "broadcast" });

      expect(a1.sendText).toHaveBeenCalledWith("c1", "broadcast", expect.objectContaining({}));
      expect(a2.sendText).toHaveBeenCalledWith("c1", "broadcast", expect.objectContaining({}));
    });

    it("sends file when filePath is set", async () => {
      const a1 = makeAdapter("a1");
      bus.register(a1);

      await bus.send({ adapterId: "a1", chatId: "c1" }, { filePath: "/tmp/file.txt" });

      expect(a1.sendFile).toHaveBeenCalledWith("c1", "/tmp/file.txt", expect.objectContaining({}));
      expect(a1.sendText).not.toHaveBeenCalled();
    });

    it("throws when specified adapter not found", async () => {
      await expect(
        bus.send({ adapterId: "nonexistent", chatId: "c1" }, { text: "hi" })
      ).rejects.toThrow("Adapter nonexistent not found");
    });

    it("forwards threadId from target to sendText opts", async () => {
      const a1 = makeAdapter("a1");
      bus.register(a1);

      await bus.send({ adapterId: "a1", chatId: "c1", threadId: "t1" }, { text: "hi" });

      expect(a1.sendText).toHaveBeenCalledWith("c1", "hi", expect.objectContaining({ threadId: "t1" }));
    });
  });

  describe("approval race", () => {
    it("resolves with first response (approve wins)", async () => {
      const a1 = makeAdapter("a1", "telegram");
      const a2 = makeAdapter("a2", "slack");

      a1.sendApproval = vi.fn(async (_prompt, cb, _signal) => {
        setTimeout(() => cb("approve"), 10);
        return { cancel: vi.fn() };
      });
      a2.sendApproval = vi.fn(async (_prompt, _cb, _signal) => {
        // a2 never responds
        return { cancel: vi.fn() };
      });

      bus.register(a1);
      bus.register(a2);

      const result = await bus.requestApproval({ tool_name: "Bash", description: "Allow this?" });
      expect(result.decision).toBe("approve");
      expect(result.respondedBy.channelType).toBe("telegram");
      expect(result.respondedBy.userId).toBe("a1");
    });

    it("resolves with first response (deny wins race)", async () => {
      const a1 = makeAdapter("a1", "telegram");
      const a2 = makeAdapter("a2", "slack");

      a1.sendApproval = vi.fn(async (_prompt, cb, _signal) => {
        setTimeout(() => cb("deny"), 5);
        return { cancel: vi.fn() };
      });
      a2.sendApproval = vi.fn(async (_prompt, cb, _signal) => {
        setTimeout(() => cb("approve"), 50);
        return { cancel: vi.fn() };
      });

      bus.register(a1);
      bus.register(a2);

      const result = await bus.requestApproval({ tool_name: "Bash", description: "Allow?" });
      expect(result.decision).toBe("deny");
    });

    it("cancels other handles when one responds", async () => {
      const cancelA2 = vi.fn();
      const a1 = makeAdapter("a1", "telegram");
      const a2 = makeAdapter("a2", "slack");

      a1.sendApproval = vi.fn(async (_prompt, cb, _signal) => {
        setTimeout(() => cb("approve"), 10);
        return { cancel: vi.fn() };
      });
      a2.sendApproval = vi.fn(async (_prompt, _cb, _signal) => {
        return { cancel: cancelA2 };
      });

      bus.register(a1);
      bus.register(a2);

      await bus.requestApproval("Allow?");

      // Give a tick for handle promises to resolve and cancel to be called
      await new Promise(r => setTimeout(r, 20));
      expect(cancelA2).toHaveBeenCalled();
    });

    it("auto-denies on timeout", async () => {
      vi.useFakeTimers();

      const a1 = makeAdapter("a1", "telegram");
      a1.sendApproval = vi.fn(async (_prompt, _cb, _signal) => {
        return { cancel: vi.fn() };
      });

      bus.register(a1);

      const promise = bus.requestApproval({ tool_name: "Bash", description: "Allow?" });
      vi.advanceTimersByTime(120_000);
      const result = await promise;

      expect(result.decision).toBe("deny");
      expect(result.respondedBy.channelType).toBe("timeout");
      expect(result.respondedBy.userId).toBe("");

      vi.useRealTimers();
    });

    it("immediately denies when no adapters registered", async () => {
      const result = await bus.requestApproval({ tool_name: "Bash", description: "Allow?" });
      expect(result.decision).toBe("deny");
      expect(result.respondedBy.channelType).toBe("none");
    });
  });
});
