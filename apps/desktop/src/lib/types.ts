export type AgentStatus = 'CREATED' | 'STARTING' | 'RUNNING' | 'STOPPING' | 'STOPPED' | 'ERROR';

export interface Agent {
  id: string;
  name: string;
  template: string;
  workdir: string;
  compose_file: string;
  status: AgentStatus;
  created_at: string;
  updated_at: string;
  last_error?: string | null;
  last_seen_container_id?: string | null;
  exit_code?: number | null;
}

export interface DockerStatus {
  installed: boolean;
  daemon_running: boolean;
  compose_available: boolean;
  version?: string;
  error?: string;
  fix_hint?: string;
}

export interface CreateAgentPayload {
  name: string;
  template: string;
  image_override?: string;
  env: Record<string, string>;
}

export interface ImportAgentPayload {
  folder_path: string;
  name?: string;
  template?: string;
}
