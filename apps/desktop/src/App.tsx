import { check } from '@tauri-apps/plugin-updater';
import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AgentDetail } from '@/pages/AgentDetail';
import { NewAgentDialog } from '@/pages/NewAgentDialog';
import { MissionsPage } from '@/pages/MissionsPage';
import { Badge, Button, Card, Input, Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui';
import { useUiStore } from '@/store/ui-store';
import { useTargetsStore } from '@/store/targets-store';

export function App() {
  const queryClient = useQueryClient();
  const selectedAgentId = useUiStore((s) => s.selectedAgentId);
  const setSelectedAgent = useUiStore((s) => s.setSelectedAgent);
  const { targets, selectedTargetId, selectTarget, addTarget, getClient } = useTargetsStore();
  const client = getClient();
  const [updateMsg, setUpdateMsg] = useState('');
  const [showTargetForm, setShowTargetForm] = useState(false);
  const [targetName, setTargetName] = useState('');
  const [targetUrl, setTargetUrl] = useState('');
  const [targetToken, setTargetToken] = useState('');

  const appInfo = useQuery({ queryKey: ['app-info', selectedTargetId], queryFn: () => client.getAppInfo() });
  const docker = useQuery({ queryKey: ['docker', selectedTargetId], queryFn: () => client.checkDocker(), refetchInterval: 8000 });
  useQuery({ queryKey: ['status-sync', selectedTargetId], queryFn: () => client.syncAllStatuses(), refetchInterval: 8000 });
  const agents = useQuery({ queryKey: ['agents', selectedTargetId], queryFn: () => client.listAgents(), refetchInterval: 3000 });

  const actionMutation = useMutation({
    mutationFn: ({ action, id }: { action: 'start' | 'stop' | 'restart'; id: string }) => {
      if (action === 'start') return client.startAgent(id);
      if (action === 'stop') return client.stopAgent(id);
      return client.restartAgent(id);
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['agents', selectedTargetId] })
  });

  const selected = useMemo(() => agents.data?.find((a) => a.id === selectedAgentId) ?? agents.data?.[0], [agents.data, selectedAgentId]);

  return (
    <main className="grid min-h-screen grid-cols-[420px_1fr] gap-4 p-4">
      <Card className="space-y-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">OpenShell</h1>
          <NewAgentDialog />
        </div>

        <div className="flex items-center gap-2">
          <select className="rounded border border-slate-700 bg-slate-950 px-2 py-1" value={selectedTargetId} onChange={(e) => selectTarget(e.target.value)}>
            {targets.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
          </select>
          <Button onClick={() => setShowTargetForm((v) => !v)}>Add Target</Button>
        </div>
        {showTargetForm ? (
          <Card className="space-y-2">
            <Input placeholder="Target name" value={targetName} onChange={(e) => setTargetName(e.target.value)} />
            <Input placeholder="Base URL" value={targetUrl} onChange={(e) => setTargetUrl(e.target.value)} />
            <Input placeholder="Bearer token" type="password" value={targetToken} onChange={(e) => setTargetToken(e.target.value)} />
            <Button onClick={async () => {
              try {
                await fetch(`${targetUrl}/v1/health`, { headers: { authorization: `Bearer ${targetToken}` } });
                addTarget({ id: crypto.randomUUID(), name: targetName, type: 'remote', base_url: targetUrl, token: targetToken });
                setShowTargetForm(false);
              } catch {
                alert('Connection test failed');
              }
            }}>Test + Save</Button>
          </Card>
        ) : null}

        <Card className={docker.data?.installed && docker.data?.daemon_running ? 'border-emerald-700 bg-emerald-950/30' : 'border-amber-700 bg-amber-950/30'}>
          {docker.data?.installed && docker.data?.daemon_running ? 'Target ready' : `Target issue: ${docker.data?.fix_hint ?? docker.data?.error ?? 'unknown'}`}
        </Card>

        <Button onClick={() => void client.syncAllStatuses().then(() => queryClient.invalidateQueries({ queryKey: ['agents', selectedTargetId] }))}>Refresh</Button>

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
              </div>
            </div>
          ))}
        </div>

        <Card className="bg-slate-950">
          <h2 className="mb-2 font-semibold">Settings / Updates</h2>
          <p className="text-xs">Version: {appInfo.data?.version ?? '-'}</p>
          <Button className="mt-2 w-full" onClick={async () => {
            const update = await check();
            if (!update) { setUpdateMsg('No updates available'); return; }
            setUpdateMsg(`Update found: ${update.version}. Downloading...`);
            await update.downloadAndInstall();
            setUpdateMsg('Update installed. Restart app to apply.');
          }}>Check for Updates</Button>
          {updateMsg ? <p className="mt-2 text-xs text-slate-300">{updateMsg}</p> : null}
        </Card>
      </Card>

      <Tabs defaultValue="agents">
        <TabsList>
          <TabsTrigger value="agents">Agents</TabsTrigger>
          <TabsTrigger value="missions">Missions</TabsTrigger>
        </TabsList>
        <TabsContent value="agents" className="pt-4">{selected ? <AgentDetail agent={selected} /> : <Card>No agents yet.</Card>}</TabsContent>
        <TabsContent value="missions" className="pt-4"><MissionsPage /></TabsContent>
      </Tabs>
    </main>
  );
}
