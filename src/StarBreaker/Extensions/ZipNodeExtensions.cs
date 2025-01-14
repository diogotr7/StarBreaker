﻿using System.Globalization;
using Humanizer;
using StarBreaker.P4k;

namespace StarBreaker.Extensions;

public static class ZipNodeExtensions
{
    public static string GetSize(this IP4kNode x)
    {
        if (x is not P4kFileNode file)
            return "";

        return ((long?)file.ZipEntry?.UncompressedSize)?.Bytes().ToString() ?? "";
    }

    public static string GetDate(this IP4kNode x)
    {
        if (x is not P4kFileNode file)
            return "";

        return file.ZipEntry?.LastModified.ToString("s", CultureInfo.InvariantCulture) ?? "";
    }

    public static string GetName(this IP4kNode x)
    {
        return x switch
        {
            P4kFileNode file => file.ZipEntry.Name.Split('\\').Last(),
            P4kDirectoryNode dir => dir.Name,
            _ => "",
        };
    }

    public static ICollection<IP4kNode> GetChildren(this IP4kNode x)
    {
        if (x is not P4kDirectoryNode dir)
            return Array.Empty<IP4kNode>();

        return dir.Children.Values;
    }

    public static ulong SizeOrZero(this IP4kNode x)
    {
        if (x is not P4kFileNode file)
            return 0;

        return file.ZipEntry?.UncompressedSize ?? 0;
    }
}