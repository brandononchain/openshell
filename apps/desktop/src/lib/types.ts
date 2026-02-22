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
  image?: string;
}

export interface DockerStatus {
  installed: boolean;
  compose: 'docker compose' | 'docker-compose' | null;
  version?: string;
  error?: string;
}

export interface CreateAgentPayload {
  name: string;
  template: string;
  image_override?: string;
  env: Record<string, string>;
}
