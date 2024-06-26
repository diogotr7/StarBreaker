using System.Runtime.InteropServices;

namespace StarBreaker.P4k;

//code from DotNext.IO

/// <summary>
/// Represents read-only view over the portion of underlying stream.
/// </summary>
/// <remarks>
/// The segmentation is supported only for seekable streams.
/// </remarks>
/// <param name="stream">The underlying stream represented by the segment.</param>
/// <param name="leaveOpen"><see langword="true"/> to leave <paramref name="stream"/> open after the object is disposed; otherwise, <see langword="false"/>.</param>
public sealed class StreamSegment(Stream stream, bool leaveOpen = true) : Stream
{
    private long length = stream.Length, offset;

    /// <summary>
    /// Gets underlying stream.
    /// </summary>
    public Stream BaseStream => stream;

    /// <summary>
    /// Establishes segment bounds.
    /// </summary>
    /// <remarks>
    /// This method modifies <see cref="Stream.Position"/> property of the underlying stream.
    /// </remarks>
    /// <param name="offset">The offset in the underlying stream.</param>
    /// <param name="length">The length of the segment.</param>
    /// <exception cref="ArgumentOutOfRangeException"><paramref name="length"/> is larger than the remaining length of the underlying stream; or <paramref name="offset"/> if greater than the length of the underlying stream.</exception>
    public void Adjust(long offset, long length)
    {
        ArgumentOutOfRangeException.ThrowIfGreaterThan((ulong)offset, (ulong)stream.Length, nameof(offset));
        ArgumentOutOfRangeException.ThrowIfGreaterThan((ulong)length, (ulong)(stream.Length - offset), nameof(length));

        this.length = length;
        this.offset = offset;
        stream.Position = offset;
    }

    /// <inheritdoc/>
    public override bool CanRead => stream.CanRead;

    /// <inheritdoc/>
    public override bool CanSeek => stream.CanSeek;

    /// <inheritdoc/>
    public override bool CanWrite => false;

    /// <inheritdoc/>
    public override long Length => length;

    /// <inheritdoc/>
    public override long Position
    {
        get => stream.Position - offset;
        set
        {
            ArgumentOutOfRangeException.ThrowIfGreaterThan((ulong)value, (ulong)length, nameof(value));

            stream.Position = offset + value;
        }
    }

    private long RemainingBytes => length - Position;

    /// <inheritdoc/>
    public override void Flush() => stream.Flush();

    /// <inheritdoc/>
    public override Task FlushAsync(CancellationToken token = default) => stream.FlushAsync(token);

    /// <inheritdoc/>
    public override bool CanTimeout => stream.CanTimeout;

    /// <inheritdoc/>
    public override int ReadByte()
        => Position < length ? stream.ReadByte() : -1;

    /// <inheritdoc/>
    public override void WriteByte(byte value) => throw new NotSupportedException();

    /// <inheritdoc/>
    public override int Read(byte[] buffer, int offset, int count)
    {
        ValidateBufferArguments(buffer, offset, count);

        return stream.Read(buffer, offset, (int)Math.Min(count, RemainingBytes));
    }

    /// <inheritdoc/>
    public override int Read(Span<byte> buffer)
        => stream.Read(buffer.TrimLength(int.CreateSaturating(RemainingBytes)));

    /// <inheritdoc/>
    public override IAsyncResult BeginRead(byte[] buffer, int offset, int count, AsyncCallback? callback, object? state)
    {
        count = (int)Math.Min(count, RemainingBytes);
        return stream.BeginRead(buffer, offset, count, callback, state);
    }

    /// <inheritdoc/>
    public override int EndRead(IAsyncResult asyncResult) => stream.EndRead(asyncResult);

    /// <inheritdoc/>
    public override Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken token = default)
        => stream.ReadAsync(buffer, offset, (int)Math.Min(count, RemainingBytes), token);

    /// <inheritdoc/>
    public override ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken token = default)
        => stream.ReadAsync(buffer.TrimLength(int.CreateSaturating(RemainingBytes)), token);

    /// <inheritdoc/>
    public override long Seek(long offset, SeekOrigin origin)
    {
        var newPosition = origin switch
        {
            SeekOrigin.Begin => offset,
            SeekOrigin.Current => Position + offset,
            SeekOrigin.End => length + offset,
            _ => throw new ArgumentOutOfRangeException(nameof(origin))
        };

        if (newPosition < 0L)
            throw new IOException();

        ArgumentOutOfRangeException.ThrowIfGreaterThan(newPosition, length, nameof(offset));

        Position = newPosition;
        return newPosition;
    }

    /// <inheritdoc/>
    public override void SetLength(long value)
    {
        ArgumentOutOfRangeException.ThrowIfGreaterThan((ulong)value, (ulong)(stream.Length - stream.Position), nameof(value));

        length = value;
    }

    /// <inheritdoc/>
    public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

    /// <inheritdoc/>
    public override void Write(ReadOnlySpan<byte> buffer) => throw new NotSupportedException();

    /// <inheritdoc/>
    public override Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken token = default) => Task.FromException(new NotSupportedException());

    /// <inheritdoc/>
    public override ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default)
        => ValueTask.FromException(new NotSupportedException());

    /// <inheritdoc/>
    public override IAsyncResult BeginWrite(byte[] buffer, int offset, int count, AsyncCallback? callback, object? state)
        => throw new NotSupportedException();

    /// <inheritdoc/>
    public override void EndWrite(IAsyncResult asyncResult) => throw new InvalidOperationException();

    /// <inheritdoc/>
    public override int ReadTimeout
    {
        get => stream.ReadTimeout;
        set => stream.ReadTimeout = value;
    }

    /// <inheritdoc/>
    public override int WriteTimeout
    {
        get => stream.WriteTimeout;
        set => stream.WriteTimeout = value;
    }

    /// <inheritdoc/>
    protected override void Dispose(bool disposing)
    {
        if (disposing && !leaveOpen)
            stream.Dispose();
        base.Dispose(disposing);
    }

    /// <inheritdoc/>
    public override async ValueTask DisposeAsync()
    {
        if (!leaveOpen)
            await stream.DisposeAsync().ConfigureAwait(false);
        await base.DisposeAsync().ConfigureAwait(false);
    }
}

public static class Extensions
{
    /// <summary>
    /// Trims the span to specified length if it exceeds it.
    /// If length is less that <paramref name="maxLength" /> then the original span returned.
    /// </summary>
    /// <typeparam name="T">The type of items in the span.</typeparam>
    /// <param name="span">A contiguous region of arbitrary memory.</param>
    /// <param name="maxLength">Maximum length.</param>
    /// <returns>Trimmed span.</returns>
    /// <exception cref="ArgumentOutOfRangeException"><paramref name="maxLength"/> is less than zero.</exception>
    public static Span<T> TrimLength<T>(this Span<T> span, int maxLength)
    {
        switch (maxLength)
        {
            case < 0:
                throw new ArgumentOutOfRangeException(nameof(maxLength));
            case 0:
                span = default;
                break;
            default:
                if (span.Length > maxLength)
                    span = MemoryMarshal.CreateSpan(ref MemoryMarshal.GetReference(span), maxLength);
                break;
        }

        return span;
    }

    /// <summary>
    /// Trims the memory block to specified length if it exceeds it.
    /// If length is less that <paramref name="maxLength" /> then the original block returned.
    /// </summary>
    /// <typeparam name="T">The type of items in the span.</typeparam>
    /// <param name="memory">A contiguous region of arbitrary memory.</param>
    /// <param name="maxLength">Maximum length.</param>
    /// <returns>Trimmed memory block.</returns>
    public static Memory<T> TrimLength<T>(this Memory<T> memory, int maxLength) => memory.Length <= maxLength ? memory : memory.Slice(0, maxLength);
}