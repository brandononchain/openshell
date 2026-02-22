import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Agent, CreateAgentPayload, DockerStatus, ImportAgentPayload } from './types';

export const api = {
  getAppInfo: () => invoke<{ app_data_dir: string; platform: string }>('get_app_info'),
  checkDocker: () => invoke<DockerStatus>('check_docker'),
  listAgents: () => invoke<Agent[]>('list_agents'),
  createAgent: (payload: CreateAgentPayload) => invoke<Agent>('create_agent', { payload }),
  deleteAgent: (agentId: string, downWithVolumes: boolean) => invoke<void>('delete_agent', { agentId, downWithVolumes }),
  duplicateAgent: (agentId: string, newName: string) => invoke<Agent>('duplicate_agent', { agentId, newName }),
  importAgent: (payload: ImportAgentPayload) => invoke<Agent>('import_agent', { payload }),
  startAgent: (agentId: string) => invoke<void>('start_agent', { agentId }),
  stopAgent: (agentId: string) => invoke<void>('stop_agent', { agentId }),
  restartAgent: (agentId: string) => invoke<void>('restart_agent', { agentId }),
  syncAgentStatus: (agentId: string) => invoke<void>('sync_agent_status', { agentId }),
  syncAllStatuses: () => invoke<void>('sync_all_statuses'),
  streamLogsStart: (agentId: string) => invoke<void>('stream_logs_start', { agentId }),
  streamLogsStop: (agentId: string) => invoke<void>('stream_logs_stop', { agentId }),
  runOpsCommand: (agentId: string, cmd: string) => invoke<{ stdout: string; stderr: string; code: number }>('run_ops_command', { agentId, cmd }),
  openAgentFolder: (agentId: string) => invoke<void>('open_agent_folder', { agentId }),
  getAgentConfig: (agentId: string) => invoke<{ env: string; compose: string; metadata: string }>('get_agent_config', { agentId }),
  resetAppData: () => invoke<void>('reset_app_data'),
  onLogLine: (agentId: string, cb: (line: string, stream: string) => void) =>
    listen<{ agent_id: string; line: string; stream: string }>('agent-log-line', (event) => {
      if (event.payload.agent_id === agentId) cb(event.payload.line, event.payload.stream);
    })
};
