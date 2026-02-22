import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { AgentDetail } from '@/pages/AgentDetail';
import { NewAgentDialog } from '@/pages/NewAgentDialog';
import { Badge, Button, Card, Dialog, DialogContent, DialogTrigger, Input } from '@/components/ui';
import { useUiStore } from '@/store/ui-store';
import type { Agent } from '@/lib/types';

export function App() {
  const queryClient = useQueryClient();
  const selectedAgentId = useUiStore((s) => s.selectedAgentId);
  const setSelectedAgent = useUiStore((s) => s.setSelectedAgent);
  const [dupName, setDupName] = useState('');
  const [importPath, setImportPath] = useState('');
  const [importName, setImportName] = useState('Imported Agent');
  const [removeVolumes, setRemoveVolumes] = useState(false);
  const [target, setTarget] = useState<Agent | null>(null);

  const appInfo = useQuery({ queryKey: ['app-info'], queryFn: api.getAppInfo });
  const docker = useQuery({ queryKey: ['docker'], queryFn: api.checkDocker, refetchInterval: 8000 });
  const syncQuery = useQuery({
    queryKey: ['status-sync'],
    queryFn: async () => {
      await api.syncAllStatuses();
      return true;
    },
    refetchInterval: 8000
  });
  const agents = useQuery({ queryKey: ['agents', syncQuery.dataUpdatedAt], queryFn: api.listAgents, refetchInterval: 3000 });

  const actionMutation = useMutation({
    mutationFn: ({ action, id }: { action: 'start' | 'stop' | 'restart'; id: string }) => {
      if (action === 'start') return api.startAgent(id);
      if (action === 'stop') return api.stopAgent(id);
      return api.restartAgent(id);
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['agents'] })
  });

  const selected = useMemo(() => agents.data?.find((a) => a.id === selectedAgentId) ?? agents.data?.[0], [agents.data, selectedAgentId]);

  const dockerBanner = () => {
    if (!docker.data) return null;
    if (!docker.data.installed) {
      return <Card className="border-red-700 bg-red-950/30 text-red-200">Docker missing. {docker.data.fix_hint}</Card>;
    }
    if (!docker.data.compose_available) {
      return <Card className="border-red-700 bg-red-950/30 text-red-200">Compose missing. {docker.data.fix_hint}</Card>;
    }
    if (!docker.data.daemon_running) {
      return <Card className="border-amber-700 bg-amber-950/30 text-amber-200">Docker installed but not running. {docker.data.fix_hint}</Card>;
    }
    return <Card className="border-emerald-700 bg-emerald-950/30 text-emerald-200">Docker ready ({docker.data.compose_bin})</Card>;
  };

  return (
    <main className="grid min-h-screen grid-cols-[400px_1fr] gap-4 p-4">
      <Card className="space-y-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">OpenShell</h1>
          <NewAgentDialog />
        </div>

        {dockerBanner()}

        <div className="flex gap-2">
          <Button onClick={() => void api.syncAllStatuses().then(() => queryClient.invalidateQueries({ queryKey: ['agents'] }))}>Refresh</Button>
          <Dialog>
            <DialogTrigger asChild><Button>Import</Button></DialogTrigger>
            <DialogContent>
              <h3 className="mb-2 font-semibold">Import Agent</h3>
              <Input placeholder="Folder path" value={importPath} onChange={(e) => setImportPath(e.target.value)} />
              <Input className="mt-2" placeholder="Agent name" value={importName} onChange={(e) => setImportName(e.target.value)} />
              <Button className="mt-3 w-full" onClick={async () => { await api.importAgent(importPath, importName); await queryClient.invalidateQueries({ queryKey: ['agents'] }); }}>Import Agent</Button>
            </DialogContent>
          </Dialog>
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

                <Dialog>
                  <DialogTrigger asChild><Button onClick={() => { setTarget(agent); setDupName(`${agent.name}-copy`); }}>Duplicate</Button></DialogTrigger>
                  <DialogContent>
                    <h3 className="mb-2 font-semibold">Duplicate Agent</h3>
                    <Input value={dupName} onChange={(e) => setDupName(e.target.value)} />
                    <Button className="mt-3 w-full" onClick={async () => { if (!target) return; await api.duplicateAgent(target.id, dupName); await queryClient.invalidateQueries({ queryKey: ['agents'] }); }}>Create Duplicate</Button>
                  </DialogContent>
                </Dialog>

                <Dialog>
                  <DialogTrigger asChild><Button className="border-red-800 text-red-300" onClick={() => setTarget(agent)}>Delete</Button></DialogTrigger>
                  <DialogContent>
                    <h3 className="mb-2 font-semibold text-red-300">Delete Agent</h3>
                    <label className="mb-3 flex items-center gap-2 text-sm"><input type="checkbox" checked={removeVolumes} onChange={(e) => setRemoveVolumes(e.target.checked)} /> remove volumes</label>
                    <Button className="w-full border-red-800 text-red-300" onClick={async () => { if (!target) return; await api.deleteAgent(target.id, removeVolumes); await queryClient.invalidateQueries({ queryKey: ['agents'] }); }}>Confirm Delete</Button>
                  </DialogContent>
                </Dialog>
              </div>
            </div>
          ))}
        </div>

        <Card className="bg-slate-950">
          <h2 className="mb-2 font-semibold">Settings</h2>
          <p className="text-xs">Docker: {docker.data?.version ?? docker.data?.error ?? 'Checking...'}</p>
          <p className="text-xs">App Data: {appInfo.data?.app_data_dir ?? '...'}</p>
          <Button className="mt-2 w-full border-red-800 text-red-300 hover:bg-red-950" onClick={async () => { await api.resetAppData(); await queryClient.invalidateQueries({ queryKey: ['agents'] }); }}>Reset app data</Button>
        </Card>
      </Card>

      <section>{selected ? <AgentDetail agent={selected} /> : <Card>No agents yet. Create one to begin.</Card>}</section>
    </main>
  );
}
