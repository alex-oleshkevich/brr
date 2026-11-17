import { goto } from '$app/navigation';
import { ApiError } from '$lib/api/client';
import { fieldLabel } from '$lib/api/errors';

export type ToastTone = 'neutral' | 'success' | 'warning' | 'danger';
export type ToastOptions = {
  tone: ToastTone;
  title: string;
  description?: string;
  action?: { label: string; onclick: () => void };
  timeoutMs?: number;
};
export type ToastItem = ToastOptions & { id: number };

const MAX_VISIBLE = 3;

class Toasts {
  items = $state<ToastItem[]>([]);
  private nextId = 1;

  show(options: ToastOptions): void {
    this.items = [...this.items, { ...options, id: this.nextId++ }].slice(-MAX_VISIBLE);
  }

  dismiss(id: number): void {
    this.items = this.items.filter((item) => item.id !== id);
  }

  success(title: string, description?: string): void {
    this.show({ tone: 'success', title, description });
  }

  error(title: string, description?: string): void {
    this.show({ tone: 'danger', title, description });
  }

  apiError(err: unknown, title = 'Something went wrong'): void {
    if (!(err instanceof ApiError)) return this.error(title);

    // Without a form field beside it, a bare reason ("Use 1 to 64 characters…") needs to say what it is about.
    const description = err.code === 'InvalidArgument' && err.field ? `${fieldLabel(err.field)}: ${err.message}` : err.message;
    this.show({
      tone: 'danger',
      title: err.offline ? "Can't reach the brr daemon" : title,
      description: err.offline ? 'Check that it is running, then try again.' : description,
      action: err.code === 'BuildMissing' ? { label: 'Open Browser versions', onclick: () => void goto('/versions') } : undefined,
    });
  }
}

export const toast = new Toasts();
