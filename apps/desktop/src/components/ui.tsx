import * as React from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import * as TabsPrimitive from '@radix-ui/react-tabs';
import { cn } from '@/lib/utils';

export function Button(props: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...props}
      className={cn(
        'rounded-md border border-slate-600 bg-slate-800 px-3 py-2 text-sm hover:bg-slate-700 disabled:opacity-50',
        props.className
      )}
    />
  );
}

export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div {...props} className={cn('rounded-lg border border-slate-800 bg-slate-900 p-4', className)} />;
}

export function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={cn('w-full rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-sm', props.className)} />;
}

export function Badge({ className, ...props }: React.HTMLAttributes<HTMLSpanElement>) {
  return <span {...props} className={cn('inline-flex rounded-full border border-slate-700 px-2 py-0.5 text-xs', className)} />;
}

export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;
export function DialogContent({ className, ...props }: DialogPrimitive.DialogContentProps) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 bg-black/60" />
      <DialogPrimitive.Content
        {...props}
        className={cn('fixed left-1/2 top-1/2 w-[640px] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-slate-700 bg-slate-900 p-5', className)}
      />
    </DialogPrimitive.Portal>
  );
}

export const Tabs = TabsPrimitive.Root;
export const TabsContent = TabsPrimitive.Content;
export const TabsList = ({ className, ...props }: TabsPrimitive.TabsListProps) => (
  <TabsPrimitive.List {...props} className={cn('inline-flex gap-2 rounded bg-slate-800 p-1', className)} />
);
export const TabsTrigger = ({ className, ...props }: TabsPrimitive.TabsTriggerProps) => (
  <TabsPrimitive.Trigger {...props} className={cn('rounded px-3 py-1 text-sm data-[state=active]:bg-slate-700', className)} />
);
