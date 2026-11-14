export type ConfirmOptions = {
  title: string;
  description: string;
  confirmLabel: string;
  tone?: 'primary' | 'danger';
};

class ConfirmState {
  current = $state<(ConfirmOptions & { resolve: (ok: boolean) => void }) | null>(null);
}

export const confirmState = new ConfirmState();

export function confirm(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    confirmState.current = { ...options, resolve };
  });
}
