import { Component, type ErrorInfo, type ReactNode } from "react";
import { Button } from "./ui/button";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ShellX UI error", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-4 bg-background p-8 text-center text-foreground">
          <div className="text-sm font-semibold">界面出现异常</div>
          <pre className="max-h-40 max-w-lg overflow-auto rounded-lg border border-border bg-card p-3 text-left font-mono text-[10px] leading-relaxed text-muted-foreground">
            {this.state.error.message}
          </pre>
          <div className="flex gap-2">
            <Button onClick={() => this.setState({ error: null })}>重试</Button>
            <Button
              variant="outline"
              onClick={() => window.location.reload()}
            >
              重新加载
            </Button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
