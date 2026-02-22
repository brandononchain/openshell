import { useState } from 'react';
import { Button, Card, Input } from '@/components/ui';

export function MissionsPage() {
  const [json, setJson] = useState('[{"type":"start_agents","payload":{"ids":[]}}]');
  const [history, setHistory] = useState<string[]>([]);

  return (
    <Card className="space-y-3">
      <h3 className="text-lg font-semibold">Missions</h3>
      <p className="text-xs text-slate-400">Basic JSON editor + step builder placeholder (server-backed endpoints available in remote mode).</p>
      <Input value={json} onChange={(e) => setJson(e.target.value)} />
      <div className="flex gap-2">
        <Button onClick={() => setHistory((h) => [`Ran mission at ${new Date().toISOString()}`, ...h].slice(0, 20))}>Run now</Button>
      </div>
      <pre className="h-40 overflow-auto rounded border border-slate-700 bg-slate-950 p-2 text-xs">{history.join('\n') || 'No runs yet'}</pre>
    </Card>
  );
}
