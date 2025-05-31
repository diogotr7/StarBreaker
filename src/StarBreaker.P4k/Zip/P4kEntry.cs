using System.Diagnostics;
using System.Runtime.InteropServices;

namespace StarBreaker.P4k;

[DebuggerDisplay("{Name}")]
public sealed class P4kEntry
{
    private readonly uint _dosDateTime;

    public string Name { get; }
    public ulong CompressedSize { get; }
    public ulong UncompressedSize { get; }
    public ushort CompressionMethod { get; }
    public bool IsCrypted { get; }
    public ulong Offset { get; }
    public DateTime LastModified => FromDosDateTime(_dosDateTime);
    public uint Crc32 { get; }

    public P4kEntry(
        string name,
        ulong compressedSize,
        ulong uncompressedSize,
        ushort compressionMethod,
        bool isCrypted,
        ulong offset,
        uint lastModifiedDateTime,
        uint crc32
    )
    {
        Name = name.Replace('/', '\\'); //disgusting
        CompressedSize = compressedSize;
        UncompressedSize = uncompressedSize;
        CompressionMethod = compressionMethod;
        IsCrypted = isCrypted;
        Offset = offset;
        _dosDateTime = lastModifiedDateTime;
        Crc32 = crc32;
    }

    //https://source.dot.net/#System.IO.Compression/System/IO/Compression/ZipHelper.cs,76523e345de18cc8
    private static DateTime FromDosDateTime(uint dateTime)
    {
        var year = (int)(1980 + (dateTime >> 25));
        var month = (int)((dateTime >> 21) & 0xF);
        var day = (int)((dateTime >> 16) & 0x1F);
        var hour = (int)((dateTime >> 11) & 0x1F);
        var minute = (int)((dateTime >> 5) & 0x3F);
        var second = (int)((dateTime & 0x001F) * 2);

        return new DateTime(year, month, day, hour, minute, second, 0);
    }

    public string RelativeOutputPath
    {
        get
        {
            //replace windows path separators with platform ones
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                // On Windows, we can keep the forward slashes as they are
                return Name;

            return Name.Replace('\\', Path.DirectorySeparatorChar);
        }
    }
}