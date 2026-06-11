export type Transport = "stdio" | "sse";

export interface ServerEntry {
  id: string;
  name: string;
  transport: Transport;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  active: boolean;
}

export interface Profile {
  id: string;
  name: string;
  active_server_ids: string[];
}

export interface Config {
  servers: ServerEntry[];
  profiles: Profile[];
  active_profile: string | null;
}

export interface ToolWithServer {
  name: string;
  server_id: string;
  server_name: string;
  description?: string;
}

export interface ServerStatus {
  id: string;
  active: boolean;
  connected: boolean;
  tool_count: number;
}

export interface ProxyStatus {
  state: "starting" | "running" | "error";
  port: number;
  message: string;
}

export interface ClaudeStatus {
  code: boolean;
  desktop: boolean;
}
