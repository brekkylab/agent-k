/** @vitest-environment jsdom */
import { describe, expect, it } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { useZoom } from '../useZoom';

describe('useZoom', () => {
  it('starts at fit (1)', () => {
    const { result } = renderHook(() => useZoom());
    expect(result.current.scale).toBe(1);
    expect(result.current.canZoomIn).toBe(true);
    expect(result.current.canZoomOut).toBe(true);
  });

  it('clamps within [0.25, 5]', () => {
    const { result } = renderHook(() => useZoom());
    for (let i = 0; i < 40; i++) act(() => result.current.zoomIn());
    expect(result.current.scale).toBe(5);
    expect(result.current.canZoomIn).toBe(false);
    for (let i = 0; i < 60; i++) act(() => result.current.zoomOut());
    expect(result.current.scale).toBe(0.25);
    expect(result.current.canZoomOut).toBe(false);
  });

  it('reset returns to the fit baseline', () => {
    const { result } = renderHook(() => useZoom());
    act(() => result.current.zoomIn());
    expect(result.current.scale).not.toBe(1);
    act(() => result.current.reset());
    expect(result.current.scale).toBe(1);
  });

  it('toggle flips between fit and 2×', () => {
    const { result } = renderHook(() => useZoom());
    act(() => result.current.toggle());
    expect(result.current.scale).toBe(2);
    act(() => result.current.toggle());
    expect(result.current.scale).toBe(1);
  });
});
