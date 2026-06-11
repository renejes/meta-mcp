import { invoke } from "@tauri-apps/api/core";
import type {
  ClaudeStatus,
  Config,
  Profile,
  ProxyStatus,
  ServerEntry,
  ServerStatus,
  ToolWithServer,
} from "./types";

export const api = {
  getConfig: () => invoke<Config>("get_config"),
  getProxyStatus: () => invoke<ProxyStatus>("get_proxy_status"),
  saveServer: (server: ServerEntry) => invoke<void>("save_server", { server }),
  deleteServer: (id: string) => invoke<void>("delete_server", { id }),
  setServerActive: (id: string, active: boolean) =>
    invoke<void>("set_server_active", { id, active }),
  saveProfile: (profile: Profile) => invoke<void>("save_profile", { profile }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  setActiveProfile: (profileId: string | null) =>
    invoke<void>("set_active_profile", { profileId }),
  importClaudeConfig: (path: string) =>
    invoke<ServerEntry[]>("import_claude_config", { path }),
  getToolList: () => invoke<ToolWithServer[]>("get_tool_list"),
  getServerStatus: () => invoke<ServerStatus[]>("get_server_status"),
  defaultClaudeConfigPath: () => invoke<string>("default_claude_config_path"),
  getClaudeStatus: () => invoke<ClaudeStatus>("get_claude_status"),
  setClaudeCode: (enabled: boolean) =>
    invoke<void>("set_claude_code", { enabled }),
  setClaudeDesktop: (enabled: boolean) =>
    invoke<void>("set_claude_desktop", { enabled }),
};
