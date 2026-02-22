import { check } from '@tauri-apps/plugin-updater';
import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { AgentDetail } from '@/pages/AgentDetail';
import { NewAgentDialog } from '@/pages/NewAgentDialog';
import { Badge, Button, Card } from '@/components/ui';
import { useUiStore } from '@/store/ui-store';

export function App() {
  const queryClient = useQueryClient();
  const selectedAgentId = useUiStore((s) => s.selectedAgentId);
  const setSelectedAgent = useUiStore((s) => s.setSelectedAgent);
  const [updateMsg, setUpdateMsg] = useState('');

  const appInfo = useQuery({ queryKey: ['app-info'], queryFn: api.getAppInfo });
  const docker = useQuery({ queryKey: ['docker'], queryFn: api.checkDocker, refetchInterval: 8000 });
  useQuery({ queryKey: ['status-sync'], queryFn: api.syncAllStatuses, refetchInterval: 8000 });
  const agents = useQuery({ queryKey: ['agents'], queryFn: api.listAgents, refetchInterval: 3000 });

  const actionMutation = useMutation({
    mutationFn: ({ action, id }: { action: 'start' | 'stop' | 'restart'; id: string }) => {
      if (action === 'start') return api.startAgent(id);
      if (action === 'stop') return api.stopAgent(id);
      return api.restartAgent(id);
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['agents'] })
  });

  const selected = useMemo(() => agents.data?.find((a) => a.id === selectedAgentId) ?? agents.data?.[0], [agents.data, selectedAgentId]);

  return (
    <main className="grid min-h-screen grid-cols-[380px_1fr] gap-4 p-4">
      <Card className="space-y-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">OpenShell</h1>
          <NewAgentDialog />
        </div>

        {!docker.data?.installed ? <Card className="border-red-700 bg-red-950/30">Docker missing. {docker.data?.fix_hint}</Card> : null}
        {docker.data?.installed && !docker.data?.compose_available ? <Card className="border-red-700 bg-red-950/30">Compose missing. {docker.data?.fix_hint}</Card> : null}
        {docker.data?.installed && docker.data?.compose_available && !docker.data?.daemon_running ? <Card className="border-amber-700 bg-amber-950/30">Docker installed but not running. {docker.data?.fix_hint}</Card> : null}
        {docker.data?.installed && docker.data?.compose_available && docker.data?.daemon_running ? <Card className="border-emerald-700 bg-emerald-950/30">Docker ready</Card> : null}

        <div className="flex gap-2">
          <Button onClick={() => void api.syncAllStatuses().then(() => queryClient.invalidateQueries({ queryKey: ['agents'] }))}>Refresh</Button>
          <Button onClick={async () => {
            const folder = window.prompt('Import folder path');
            const name = window.prompt('Agent name', 'Imported Agent');
            if (!folder || !name) return;
            await api.importAgent(folder, name);
            await queryClient.invalidateQueries({ queryKey: ['agents'] });
          }}>Import</Button>
        </div>

        <div className="space-y-2">
          {agents.data?.map((agent) => (
            <div key={agent.id} className="space-y-2 rounded border border-slate-800 p-3">
              <div className="flex items-center justify-between">
                <button className="text-left font-medium" onClick={() => setSelectedAgent(agent.id)}>{agent.name}</button>
                <Badge>{agent.status}</Badge>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => actionMutation.mutate({ action: 'start', id: agent.id })}>Start</Button>
                <Button onClick={() => actionMutation.mutate({ action: 'stop', id: agent.id })}>Stop</Button>
                <Button onClick={() => actionMutation.mutate({ action: 'restart', id: agent.id })}>Restart</Button>
                <Button onClick={() => api.openAgentFolder(agent.id)}>Open Folder</Button>
                <Button onClick={async () => {
                  const name = window.prompt('Duplicate as', `${agent.name}-copy`);
                  if (!name) return;
                  await api.duplicateAgent(agent.id, name);
                  await queryClient.invalidateQueries({ queryKey: ['agents'] });
                }}>Duplicate</Button>
                <Button className="border-red-800 text-red-300" onClick={async () => {
                  if (!window.confirm('Delete this agent?')) return;
                  const remove = window.confirm('Remove volumes too?');
                  await api.deleteAgent(agent.id, remove);
                  await queryClient.invalidateQueries({ queryKey: ['agents'] });
                }}>Delete</Button>
              </div>
            </div>
          ))}
        </div>

        <Card className="bg-slate-950">
          <h2 className="mb-2 font-semibold">Settings / Updates</h2>
          <p className="text-xs">Version: {appInfo.data?.version ?? '-'}</p>
          <p className="text-xs">App Data: {appInfo.data?.app_data_dir ?? '-'}</p>
          <Button className="mt-2 w-full" onClick={async () => {
            const update = await check();
            if (!update) { setUpdateMsg('No updates available'); return; }
            setUpdateMsg(`Update found: ${update.version}. Downloading...`);
            await update.downloadAndInstall();
            setUpdateMsg('Update installed. Restart app to apply.');
          }}>Check for Updates</Button>
          {updateMsg ? <p className="mt-2 text-xs text-slate-300">{updateMsg}</p> : null}
          <Button className="mt-2 w-full border-red-800 text-red-300" onClick={async () => {
            await api.resetAppData();
            await queryClient.invalidateQueries({ queryKey: ['agents'] });
          }}>Reset app data</Button>
        </Card>
      </Card>
      <section>{selected ? <AgentDetail agent={selected} /> : <Card>No agents yet. Create one to begin.</Card>}</section>
    </main>
  );
}
