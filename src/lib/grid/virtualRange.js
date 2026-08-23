export function computeVirtualRange({
  scrollTop,
  viewportHeight,
  rowHeight,
  itemCount,
  overscan,
}) {
  if (itemCount <= 0) {
    return {
      startIndex: 0,
      endIndex: 0,
      topSpacerHeight: 0,
      bottomSpacerHeight: 0,
    };
  }
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
  const endIndex = Math.min(itemCount, startIndex + visibleCount);
  const topSpacerHeight = startIndex * rowHeight;
  const bottomSpacerHeight = Math.max(0, (itemCount - endIndex) * rowHeight);
  return { startIndex, endIndex, topSpacerHeight, bottomSpacerHeight };
}

export function shouldFetchMore({
  scrollTop,
  viewportHeight,
  rowHeight,
  itemCount,
  fetchThreshold,
}) {
  const lastVisibleIndex = Math.ceil((scrollTop + viewportHeight) / rowHeight);
  const remaining = itemCount - lastVisibleIndex;
  return remaining <= fetchThreshold;
}
