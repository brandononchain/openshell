import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { Button, Dialog, DialogContent, DialogTrigger, Input } from '@/components/ui';

export function NewAgentDialog() {
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [image, setImage] = useState('');
  const [envRows, setEnvRows] = useState<Array<{ key: string; value: string }>>([{ key: '', value: '' }]);

  const mutation = useMutation({
    mutationFn: () =>
      api.createAgent({
        name,
        template: 'openclaw-default',
        image_override: image || undefined,
        env: Object.fromEntries(envRows.filter((r) => r.key).map((r) => [r.key, r.value]))
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['agents'] });
      setName('');
      setImage('');
      setEnvRows([{ key: '', value: '' }]);
    }
  });

  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button>New Agent</Button>
      </DialogTrigger>
      <DialogContent>
        <h2 className="mb-4 text-xl font-semibold">New Agent Wizard</h2>
        <div className="space-y-3">
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Agent name" />
          <Input value="openclaw-default" disabled />
          <Input value={image} onChange={(e) => setImage(e.target.value)} placeholder="Image override (optional)" />
          <div className="space-y-2">
            {envRows.map((row, idx) => (
              <div className="grid grid-cols-2 gap-2" key={idx}>
                <Input
                  placeholder="ENV_KEY"
                  value={row.key}
                  onChange={(e) => setEnvRows((prev) => prev.map((it, i) => (i === idx ? { ...it, key: e.target.value } : it)))}
                />
                <Input
                  placeholder="value"
                  value={row.value}
                  onChange={(e) => setEnvRows((prev) => prev.map((it, i) => (i === idx ? { ...it, value: e.target.value } : it)))}
                />
              </div>
            ))}
            <Button className="w-full" onClick={() => setEnvRows((prev) => [...prev, { key: '', value: '' }])}>Add Env Var</Button>
          </div>
          <Button disabled={!name || mutation.isPending} className="w-full" onClick={() => mutation.mutate()}>
            {mutation.isPending ? 'Creating...' : 'Create Agent'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
