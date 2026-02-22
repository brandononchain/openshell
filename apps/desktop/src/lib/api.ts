import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Agent, AgentStats, AgentTemplate, DockerStatus } from './types';

export const api = {
  getAppInfo: () => invoke<{ app_data_dir: string; platform: string; version: string }>('get_app_info'),
  checkDocker: () => invoke<DockerStatus>('check_docker'),
  listTemplates: () => invoke<AgentTemplate[]>('list_templates'),
  createAgentFromTemplate: (payload: { template_id: string; name: string; env_values: Record<string, string>; image_override?: string }) =>
    invoke<Agent>('create_agent_from_template', { payload }),
  listAgents: () => invoke<Agent[]>('list_agents'),
  startAgent: (agentId: string) => invoke<void>('start_agent', { agentId }),
  stopAgent: (agentId: string) => invoke<void>('stop_agent', { agentId }),
  restartAgent: (agentId: string) => invoke<void>('restart_agent', { agentId }),
  syncAllStatuses: () => invoke<void>('sync_all_statuses'),
  streamLogsStart: (agentId: string) => invoke<void>('stream_logs_start', { agentId }),
  streamLogsStop: (agentId: string) => invoke<void>('stream_logs_stop', { agentId }),
  runOpsCommand: (agentId: string, cmd: string) => invoke<{ stdout: string; stderr: string; code: number }>('run_ops_command', { agentId, cmd }),
  openAgentFolder: (agentId: string) => invoke<void>('open_agent_folder', { agentId }),
  getAgentConfig: (agentId: string) => invoke<{ env: string; compose: string; metadata: string }>('get_agent_config', { agentId }),
  deleteAgent: (agentId: string, removeVolumes: boolean) => invoke<void>('delete_agent', { agentId, removeVolumes }),
  duplicateAgent: (agentId: string, newName: string) => invoke<Agent>('duplicate_agent', { agentId, newName }),
  importAgent: (folderPath: string, name: string) => invoke<Agent>('import_agent', { folderPath, name }),
  exportAgentBundle: (agentId: string, includeSecrets: boolean) => invoke<string>('export_agent_bundle', { agentId, includeSecrets }),
  importAgentBundle: (zipPath: string) => invoke<Agent>('import_agent_bundle', { zipPath }),
  getAgentStats: (agentId: string) => invoke<AgentStats>('get_agent_stats', { agentId }),
  attachAgentShell: (agentId: string, preferredShell: string) => invoke<void>('attach_agent_shell', { agentId, preferredShell }),
  writeAgentShell: (agentId: string, data: string) => invoke<void>('write_agent_shell', { agentId, data }),
  detachAgentShell: (agentId: string) => invoke<void>('detach_agent_shell', { agentId }),
  resetAppData: () => invoke<void>('reset_app_data'),
  onLogLine: (agentId: string, cb: (line: string, stream: string) => void) =>
    listen<{ agent_id: string; line: string; stream: string }>('agent-log-line', (event) => {
      if (event.payload.agent_id === agentId) cb(event.payload.line, event.payload.stream);
    }),
  onShellLine: (agentId: string, cb: (line: string) => void) =>
    listen<{ agent_id: string; line: string }>('agent-shell-line', (event) => {
      if (event.payload.agent_id === agentId) cb(event.payload.line);
    })
};
