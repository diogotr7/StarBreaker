﻿using System.Buffers;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace StarBreaker.Common;

public static class StreamExtensions
{
    public static T Read<T>(this Stream stream) where T : unmanaged
    {
        var size = Unsafe.SizeOf<T>();

        if (size > 256)
            throw new Exception("Size is too large");

        Span<byte> span = stackalloc byte[size];

        if (stream.Read(span) != size)
            throw new Exception("Failed to read from stream");

        return MemoryMarshal.Read<T>(span);
    }

    public static void CopyAmountTo(this Stream source, Stream destination, int byteCount)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(byteCount);
        try
        {
            while (byteCount > 0)
            {
                var n = source.Read(buffer, 0, Math.Min(byteCount, buffer.Length));
                if (n == 0)
                    throw new Exception("Failed to read from stream");
                destination.Write(buffer, 0, n);
                byteCount -= n;
            }
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }
    
    public static byte[] ToArray(this Stream stream)
    {
        if (stream is MemoryStream ms)
            return ms.ToArray();

        try
        {
            var count = stream.Position;
            var buffer = new byte[count];
            stream.Position = 0;
            stream.ReadExactly(buffer, 0, buffer.Length);
            return buffer;
        }
        catch (NotSupportedException)
        {
            using var m = new MemoryStream();
            stream.CopyTo(m);
            return m.ToArray();
        }
    }
}