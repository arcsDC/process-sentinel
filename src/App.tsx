import React, { useEffect, useRef, useState } from 'react';
import { listen, invoke } from '@tauri-apps/api/event';
import { useSentinelStore } from './store';
import { ProcessState, WatchRule, LogEntry } from './types';

const App: React.FC = () => {
  const { processes, addProcess, removeProcess, updateProcess, logs, addLog, clearLogs } = useSentinelStore();
  const [newRule, setNewRule] = useState<Partial<WatchRule>>({
