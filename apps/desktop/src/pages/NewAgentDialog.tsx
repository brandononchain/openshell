import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { Button, Card, Dialog, DialogContent, DialogTrigger, Input } from '@/components/ui';

export function NewAgentDialog() {
  const queryClient = useQueryClient();
  const templates = useQuery({ queryKey: ['templates'], queryFn: api.listTemplates });
  const [selectedTemplate, setSelectedTemplate] = useState<string>('');
  const [name, setName] = useState('');
  const [imageOverride, setImageOverride] = useState('');
  const [envValues, setEnvValues] = useState<Record<string, string>>({});

  const tpl = useMemo(() => templates.data?.find((t) => t.id === selectedTemplate) ?? templates.data?.[0], [templates.data, selectedTemplate]);

  const mutation = useMutation({
    mutationFn: () => api.createAgentFromTemplate({ template_id: tpl!.id, name, env_values: envValues, image_override: imageOverride || undefined }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['agents'] });
      setName('');
      setImageOverride('');
      setEnvValues({});
    }
  });

  return (
    <Dialog>
      <DialogTrigger asChild><Button>New Agent</Button></DialogTrigger>
      <DialogContent>
        <h2 className="mb-3 text-xl font-semibold">New Agent Wizard</h2>
        <div className="mb-3 grid grid-cols-2 gap-2">
          {templates.data?.map((t) => (
            <Card key={t.id} className={`cursor-pointer ${tpl?.id === t.id ? 'border-cyan-600' : ''}`} onClick={() => setSelectedTemplate(t.id)}>
              <p className="font-medium">{t.name}</p>
              <p className="text-xs text-slate-400">{t.description}</p>
            </Card>
          ))}
        </div>
        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Agent name" />
        <Input className="mt-2" value={imageOverride} onChange={(e) => setImageOverride(e.target.value)} placeholder="Image override (optional)" />
        <div className="mt-3 space-y-2">
          {tpl?.env_schema.map((field) => (
            <Input
              key={field.key}
              type={field.secret ? 'password' : 'text'}
              placeholder={`${field.label}${field.required ? ' *' : ''}`}
              value={envValues[field.key] ?? field.default ?? ''}
              onChange={(e) => setEnvValues((prev) => ({ ...prev, [field.key]: e.target.value }))}
            />
          ))}
        </div>
        <Button className="mt-3 w-full" disabled={!name || !tpl || mutation.isPending} onClick={() => mutation.mutate()}>
          {mutation.isPending ? 'Creating...' : 'Create Agent'}
        </Button>
      </DialogContent>
    </Dialog>
  );
}
