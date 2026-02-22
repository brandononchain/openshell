import { api } from './api';
import type { Agent, AgentTemplate, DockerStatus } from './types';

export type TargetType = 'local' | 'remote';

export interface Target {
  id: string;
  name: string;
  type: TargetType;
  base_url?: string;
  token?: string;
}

export interface SupervisorClient {
  getAppInfo(): Promise<{ app_data_dir: string; platform: string; version: string }>;
  checkDocker(): Promise<DockerStatus>;
  listTemplates(): Promise<AgentTemplate[]>;
  createAgentFromTemplate(payload: { template_id: string; name: string; env_values: Record<string, string>; image_override?: string }): Promise<Agent>;
  listAgents(): Promise<Agent[]>;
  startAgent(id: string): Promise<void>;
  stopAgent(id: string): Promise<void>;
  restartAgent(id: string): Promise<void>;
  syncAllStatuses(): Promise<void>;
}

export const localClient: SupervisorClient = {
  getAppInfo: api.getAppInfo,
  checkDocker: api.checkDocker,
  listTemplates: api.listTemplates,
  createAgentFromTemplate: api.createAgentFromTemplate,
  listAgents: api.listAgents,
  startAgent: api.startAgent,
  stopAgent: api.stopAgent,
  restartAgent: api.restartAgent,
  syncAllStatuses: api.syncAllStatuses
};

function remoteFetch<T>(target: Target, path: string, init?: RequestInit): Promise<T> {
  return fetch(`${target.base_url}${path}`, {
    ...init,
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${target.token ?? ''}`,
      ...(init?.headers ?? {})
    }
  }).then(async (r) => {
    if (!r.ok) throw new Error(await r.text());
    if (r.status === 204) return undefined as T;
    return (await r.json()) as T;
  });
}

export function makeRemoteClient(target: Target): SupervisorClient {
  return {
    getAppInfo: async () => ({ app_data_dir: '-', platform: 'remote', version: 'remote' }),
    checkDocker: () => remoteFetch<DockerStatus>(target, '/v1/health').then(() => ({ installed: true, daemon_running: true, compose_available: true } as DockerStatus)),
    listTemplates: () => remoteFetch<AgentTemplate[]>(target, '/v1/templates'),
    createAgentFromTemplate: (payload) => remoteFetch<Agent>(target, '/v1/agents', { method: 'POST', body: JSON.stringify({ name: payload.name, template: payload.template_id }) }),
    listAgents: () => remoteFetch<Agent[]>(target, '/v1/agents'),
    startAgent: (id) => remoteFetch<void>(target, `/v1/agents/${id}/start`, { method: 'POST' }),
    stopAgent: (id) => remoteFetch<void>(target, `/v1/agents/${id}/stop`, { method: 'POST' }),
    restartAgent: (id) => remoteFetch<void>(target, `/v1/agents/${id}/restart`, { method: 'POST' }),
    syncAllStatuses: async () => undefined
  };
}
