"use client";

import type { FC } from "react";
import { ArrowUpIcon } from "lucide-react";
import {
  ThreadPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
} from "@assistant-ui/react";

export const Thread: FC<{ composerLeft?: React.ReactNode }> = ({ composerLeft }) => {
  return (
    <ThreadPrimitive.Root className="flex h-full flex-col bg-background">
      <ThreadPrimitive.Viewport className="relative flex flex-1 flex-col overflow-y-auto px-4">
        <ThreadPrimitive.If empty>
          <ThreadWelcome />
        </ThreadPrimitive.If>

        <ThreadPrimitive.If empty={false}>
          <ThreadPrimitive.Messages
            components={{
              UserMessage,
              AssistantMessage,
            }}
          />
          <div className="min-h-8 grow" />
        </ThreadPrimitive.If>
      </ThreadPrimitive.Viewport>

      <Composer composerLeft={composerLeft} />
    </ThreadPrimitive.Root>
  );
};

export const ThreadWelcome: FC = () => {
  return (
    <div className="mx-auto my-auto flex w-full max-w-2xl flex-grow flex-col items-center justify-center">
      <div className="flex flex-col items-center gap-3">
        <h2 className="text-xl font-semibold text-muted-foreground">
          agentwebui-test에 질문하세요
        </h2>
        <p className="text-sm text-muted-foreground text-center">
          좌상단 버튼으로 Knowledge를 활성화하면
          <br />
          문서 기반 대화가 가능합니다
        </p>
      </div>
    </div>
  );
};

export const Composer: FC<{ composerLeft?: React.ReactNode }> = ({ composerLeft }) => {
  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4 bg-background pb-4 px-4">
      <ComposerPrimitive.Root className="relative flex w-full flex-col rounded-2xl border border-input bg-background px-1 pt-2 shadow-sm transition-colors focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/50">
        <ComposerPrimitive.Input
          placeholder="메시지를 입력하세요..."
          className="mb-1 max-h-32 min-h-[60px] w-full resize-none bg-transparent px-3.5 pt-1.5 pb-3 text-base outline-none placeholder:text-muted-foreground"
          rows={1}
          autoFocus
        />
        <div className="mx-1 mb-2 flex items-center justify-between">
          <div>{composerLeft}</div>
          <ComposerPrimitive.Send className="inline-flex items-center justify-center h-8 w-8 rounded-full bg-primary text-primary-foreground hover:bg-primary/90">
            <ArrowUpIcon className="h-4 w-4" />
          </ComposerPrimitive.Send>
        </div>
      </ComposerPrimitive.Root>
    </div>
  );
};

export const AssistantMessage: FC = () => {
  return (
    <MessagePrimitive.Root className="relative mx-auto w-full max-w-2xl py-4">
      <div className="leading-7 break-words text-foreground">
        <MessagePrimitive.Content />
      </div>
    </MessagePrimitive.Root>
  );
};

export const UserMessage: FC = () => {
  return (
    <MessagePrimitive.Root className="mx-auto w-full max-w-2xl py-4">
      <div className="flex justify-end">
        <div className="rounded-2xl bg-muted px-4 py-2.5 break-words text-foreground">
          <MessagePrimitive.Content />
        </div>
      </div>
    </MessagePrimitive.Root>
  );
};
