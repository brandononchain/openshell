import { useEffect, useMemo, useRef, useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Terminal } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import 'xterm/css/xterm.css';
import { api } from '@/lib/api';
import type { Agent } from '@/lib/types';
import { Badge, Button, Card, Input, Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui';

interface LogEntry { line: string; stream: string }

export function AgentDetail({ agent }: { agent: Agent }) {
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [config, setConfig] = useState<{ env: string; compose: string; metadata: string } | null>(null);
  const [paused, setPaused] = useState(false);
  const [search, setSearch] = useState('');
  const terminalEl = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);

  const opsMutation = useMutation({
    mutationFn: (cmd: string) => api.runOpsCommand(agent.id, cmd),
    onSuccess: (result) => {
      termRef.current?.writeln(`$ ${result.code === 0 ? 'ok' : 'error'}\r\n${result.stdout}${result.stderr}`);
    },
    onError: (err) => termRef.current?.writeln(`Error: ${String(err)}`)
  });

  useEffect(() => {
    let mounted = true;
    const setup = async () => {
      await api.streamLogsStart(agent.id);
      const unlisten = await api.onLogLine(agent.id, (line, stream) => {
        if (mounted && !paused) setLogs((prev) => [...prev.slice(-800), { line, stream }]);
      });
      return unlisten;
    };
    let unlisten: (() => void) | undefined;
    void setup().then((fn) => (unlisten = fn));

    return () => {
      mounted = false;
      void api.streamLogsStop(agent.id);
      unlisten?.();
    };
  }, [agent.id, paused]);

  useEffect(() => {
    void api.getAgentConfig(agent.id).then(setConfig);
  }, [agent.id]);

  useEffect(() => {
    if (!terminalEl.current || termRef.current) return;
    const term = new Terminal({ theme: { background: '#020617' }, fontSize: 12, cursorBlink: true });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(terminalEl.current);
    fit.fit();
    term.writeln('OpenShell Ops Console');
    term.writeln('Commands: docker ps | compose up | compose down | compose logs | show config | open folder');
    termRef.current = term;
  }, []);

  const runCmd = (cmd: string) => {
    termRef.current?.writeln(`$ ${cmd}`);
    opsMutation.mutate(cmd);
  };

  const filteredLogs = useMemo(() => logs.filter((l) => l.line.toLowerCase().includes(search.toLowerCase())), [logs, search]);
  const copyLast200 = async () => {
    const txt = filteredLogs.slice(-200).map((x) => `[${x.stream}] ${x.line}`).join('\n');
    await navigator.clipboard.writeText(txt);
  };

  return (
    <Card>
      <h3 className="mb-4 text-lg font-semibold">{agent.name}</h3>
      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="terminal">Terminal</TabsTrigger>
          <TabsTrigger value="config">Config</TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-2 pt-4 text-sm">
          <p>Status: {agent.status}</p>
          <p>Container: {agent.last_seen_container_id ?? '-'}</p>
          <p>Exit code: {agent.exit_code ?? '-'}</p>
          <p>Workdir: {agent.workdir}</p>
          <p className="text-red-400">{agent.last_error}</p>
        </TabsContent>

        <TabsContent value="logs" className="pt-4">
          <div className="mb-2 flex gap-2">
            <Button onClick={() => setPaused((p) => !p)}>{paused ? 'Resume' : 'Pause'}</Button>
            <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search logs" />
            <Button onClick={() => void copyLast200()}>Copy Last 200 Lines</Button>
          </div>
          <div className="h-80 space-y-1 overflow-auto rounded border border-slate-700 bg-slate-950 p-3 text-xs">
            {filteredLogs.map((entry, idx) => (
              <div key={`${idx}-${entry.line}`} className="flex gap-2">
                <Badge className="h-fit">{entry.stream}</Badge>
                <span>{entry.line}</span>
              </div>
            ))}
          </div>
        </TabsContent>

        <TabsContent value="terminal" className="space-y-3 pt-4">
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => runCmd('docker ps')}>docker ps</Button>
            <Button onClick={() => runCmd('compose up')}>compose up</Button>
            <Button onClick={() => runCmd('compose down')}>compose down</Button>
            <Button onClick={() => runCmd('compose logs')}>compose logs</Button>
            <Button onClick={() => runCmd('show config')}>show config</Button>
            <Button onClick={() => runCmd('open folder')}>open folder</Button>
          </div>
          <div ref={terminalEl} className="h-72 rounded border border-slate-700" />
        </TabsContent>

        <TabsContent value="config" className="pt-4">
          <pre className="h-80 overflow-auto rounded border border-slate-700 bg-slate-950 p-3 text-xs">
            {config ? `${config.metadata}\n\n${config.env}\n\n${config.compose}` : 'Loading...'}
          </pre>
        </TabsContent>
      </Tabs>
    </Card>
  );
}
