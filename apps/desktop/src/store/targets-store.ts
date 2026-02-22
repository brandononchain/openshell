import { create } from 'zustand';
import { localClient, makeRemoteClient, type SupervisorClient, type Target } from '@/lib/client';

interface TargetsState {
  targets: Target[];
  selectedTargetId: string;
  addTarget: (target: Target) => void;
  selectTarget: (id: string) => void;
  getClient: () => SupervisorClient;
}

const localTarget: Target = { id: 'local', name: 'Local', type: 'local' };

export const useTargetsStore = create<TargetsState>((set, get) => ({
  targets: [localTarget],
  selectedTargetId: 'local',
  addTarget: (target) => set((s) => ({ targets: [...s.targets, target] })),
  selectTarget: (id) => set({ selectedTargetId: id }),
  getClient: () => {
    const selected = get().targets.find((t) => t.id === get().selectedTargetId) ?? localTarget;
    return selected.type === 'local' ? localClient : makeRemoteClient(selected);
  }
}));
