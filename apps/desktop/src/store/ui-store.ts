import { create } from 'zustand';

interface UiState {
  selectedAgentId: string | null;
  setSelectedAgent: (id: string | null) => void;
}

export const useUiStore = create<UiState>((set) => ({
  selectedAgentId: null,
  setSelectedAgent: (id) => set({ selectedAgentId: id })
}));
