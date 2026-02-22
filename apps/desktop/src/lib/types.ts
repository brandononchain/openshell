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
  fix_hint?: string;
  compose_bin?: string;
  version?: string;
  error?: string;
}

export interface TemplateField {
  key: string;
  label: string;
  required: boolean;
  secret: boolean;
  default?: string;
}

export interface AgentTemplate {
  id: string;
  name: string;
  description: string;
  image: string;
  env_schema: TemplateField[];
  ports: string[];
  volumes: string[];
}

export interface AgentStats {
  cpu: string;
  memory: string;
  uptime: string;
}
