import { useMemo } from 'react';
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

  const appInfo = useQuery({ queryKey: ['app-info'], queryFn: api.getAppInfo });
  const docker = useQuery({ queryKey: ['docker'], queryFn: api.checkDocker });
  const agents = useQuery({ queryKey: ['agents'], queryFn: api.listAgents, refetchInterval: 2500 });

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
    <main className="grid min-h-screen grid-cols-[360px_1fr] gap-4 p-4">
      <Card className="space-y-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">OpenShell</h1>
          <NewAgentDialog />
        </div>
        <p className="text-xs text-slate-400">Mission Control Dashboard</p>
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
              </div>
            </div>
          ))}
        </div>
        <Card className="bg-slate-950">
          <h2 className="mb-2 font-semibold">Settings</h2>
          <p className="text-xs">Docker: {docker.data?.installed ? `OK (${docker.data.compose})` : docker.data?.error ?? 'Checking...'}</p>
          <p className="text-xs">App Data: {appInfo.data?.app_data_dir ?? '...'}</p>

          <Button
            className="mt-2 w-full border-red-800 text-red-300 hover:bg-red-950"
            onClick={async () => {
              await api.resetAppData();
              await queryClient.invalidateQueries({ queryKey: ['agents'] });
            }}
          >
            Reset app data
          </Button>
        </Card>
      </Card>
      <section>{selected ? <AgentDetail agent={selected} /> : <Card>No agents yet. Create one to begin.</Card>}</section>
    </main>
  );
}
