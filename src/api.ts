import { invoke } from "@tauri-apps/api/core";

export type ToolId = "claude" | "codex" | "agy";

export interface Detail {
  label: string;
  value: string;
}

export interface ProfileInfo {
  tool: ToolId;
  name: string;
  path: string;
  isDefaultProfile: boolean;
  isSelected: boolean;
  loggedIn: boolean;
  email: string | null;
  organization: string | null;
  plan: string | null;
  expiresAt: number | null;
  refreshExpiresAt: number | null;
  scopes: string[];
  details: Detail[];
  extraArgs: string | null;
  error: string | null;
}

export interface ToolState {
  tool: ToolId;
  label: string;
  command: string;
  installed: boolean;
  exePath: string | null;
  supportsDefault: boolean;
  envVar: string | null;
  profilesRoot: string;
  envValue: string | null;
  envUnknown: boolean;
  profiles: ProfileInfo[];
}

export interface AppState {
  tools: ToolState[];
}

export interface Created {
  name: string;
  warnings: string[];
  state: AppState;
}

export const api = {
  getState: () => invoke<AppState>("get_state"),
  setDefault: (tool: ToolId, name: string) => invoke<AppState>("set_default", { tool, name }),
  createProfile: (tool: ToolId, name: string) => invoke<Created>("create_profile", { tool, name }),
  renameProfile: (tool: ToolId, old: string, newName: string) =>
    invoke<AppState>("rename_profile", { tool, old, new: newName }),
  deleteProfile: (tool: ToolId, name: string) => invoke<AppState>("delete_profile", { tool, name }),
  openTool: (tool: ToolId, name: string) => invoke<void>("open_tool", { tool, name }),
  setArgs: (tool: ToolId, name: string, args: string) => invoke<AppState>("set_profile_args", { tool, name, args }),
  openFolder: (tool: ToolId, name: string) => invoke<void>("open_folder", { tool, name }),
};

// ---------------------------------------------------------------- executáveis

export type ExeSource = "custom" | "path" | "known";

export interface ExeCandidate {
  path: string;
  source: ExeSource;
}

export interface ExeInfo {
  tool: ToolId;
  label: string;
  command: string;
  custom: string | null;
  customMissing: boolean;
  resolved: ExeCandidate | null;
  candidates: ExeCandidate[];
}

export const exeApi = {
  get: () => invoke<ExeInfo[]>("get_executables"),
  set: (tool: ToolId, path: string | null) => invoke<ExeInfo[]>("set_executable", { tool, path }),
};

export type Tone ="ok" | "warn" | "bad" | "idle";

export function sessionStatus(p: ProfileInfo): { text: string; tone: Tone } {
  if (p.error) return { text: p.error, tone: "bad" };
  if (!p.loggedIn) return { text: "Sem login", tone: "idle" };
  if (p.refreshExpiresAt && p.refreshExpiresAt <= Date.now()) return { text: "Sessão expirada — faça login de novo", tone: "bad" };
  return { text: "Conectada", tone: "ok" };
}

export function formatLeft(ms: number): string {
  const h = Math.max(0, ms - Date.now()) / 3_600_000;
  return h >= 24 ? `${(h / 24).toFixed(1)} dias` : `${h.toFixed(1)} h`;
}

export function formatDate(ms: number | null): string {
  return ms && ms > 0 ? new Date(ms).toLocaleString("pt-BR") : "—";
}

// ---------------------------------------------------------------- skills globais

export type LinkState = "on" | "off" | "conflict";

export interface GlobalSkill {
  name: string;
  description: string | null;
  path: string;
  links: { tool: ToolId; state: LinkState }[];
}

export interface LocalSkill {
  tool: ToolId;
  name: string;
  description: string | null;
  path: string;
}

export interface SkillsState {
  root: string;
  global: GlobalSkill[];
  local: LocalSkill[];
}

export const skillsApi = {
  get: () => invoke<SkillsState>("get_skills"),
  setLink: (name: string, tool: ToolId, enabled: boolean) =>
    invoke<SkillsState>("set_skill_link", { name, tool, enabled }),
  makeGlobal: (tool: ToolId, name: string) => invoke<SkillsState>("make_skill_global", { tool, name }),
  create: (name: string, description: string) => invoke<SkillsState>("create_skill", { name, description }),
  openFolder: (name?: string) => invoke<void>("open_skill_folder", { name: name ?? null }),
};

export const TOOL_LABEL: Record<ToolId, string> = { claude: "Claude Code", codex: "Codex", agy: "Antigravity" };

// ---------------------------------------------------------------- conversas (sync entre perfis)

export type Presence = "on" | "off" | "diverged";

export interface Conversation {
  id: string;
  project: string;
  cwd: string | null;
  /** null enquanto ainda não foi analisada (os títulos chegam por evento). */
  title: string | null;
  updatedAt: number;
  size: number;
  profiles: { profile: string; state: Presence }[];
}

export interface ConversationsState {
  tool: ToolId;
  supported: boolean;
  note: string | null;
  profiles: string[];
  conversations: Conversation[];
  pending: number;
}

export interface Described {
  project: string;
  id: string;
  title: string;
  cwd: string | null;
}

export interface AnalyzeProgress {
  req: number;
  tool: ToolId;
  items: Described[];
  done: number;
  finished: boolean;
}

export const convApi = {
  get: (tool: ToolId) => invoke<ConversationsState>("get_conversations", { tool }),
  set: (tool: ToolId, project: string, id: string, profile: string, enabled: boolean) =>
    invoke<ConversationsState>("set_conversation_sync", { tool, project, id, profile, enabled }),
  resync: (tool: ToolId, project: string, id: string) =>
    invoke<ConversationsState>("resync_conversation", { tool, project, id }),
  /** Lê os títulos em segundo plano; o progresso chega pelo evento "conversations-progress". */
  analyze: (tool: ToolId, req: number) => invoke<void>("analyze_conversations", { tool, req }),
};
