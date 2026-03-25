export type MeetingRole = "pro" | "con" | "arbiter" | (string & {});
export type MeetingMode = "debate" | "collab" | "discussion";

export interface MeetingConfig {
  meetingId: string;
  topic: string;
  mode: MeetingMode;
  maxRounds: number;
  repo?: string;
  angles?: string[];  // discussion mode: analysis angles for each participant
}

export interface EphemeralInstanceConfig {
  systemPrompt: string;
  workingDirectory: string;
  lightweight?: boolean;
  skipPermissions?: boolean;
  backend?: string;
}

export type RouteTarget = { kind: "instance"; name: string };
