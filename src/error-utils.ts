export interface ErrorLike {
  message?: unknown;
}

export const messageFromUnknownError = (error: unknown, fallbackMessage: string): string => {
  if (error instanceof Error && error.message.trim() !== '') {
    return error.message;
  }

  if (typeof error === 'string' && error.trim() !== '') {
    return error;
  }

  if (typeof error === 'object' && error !== null) {
    const maybeError = error as ErrorLike;
    if (typeof maybeError.message === 'string' && maybeError.message.trim() !== '') {
      return maybeError.message;
    }
  }

  return fallbackMessage;
};
